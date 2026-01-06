use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════
// JSON Schema for Refactored Lexicon
// ═══════════════════════════════════════════════════════════════════

#[derive(Deserialize)]
struct RefactoredLexiconData {
    keywords: HashMap<String, String>,
    pronouns: Vec<PronounEntry>,
    articles: HashMap<String, String>,
    auxiliaries: HashMap<String, String>,
    presupposition_triggers: HashMap<String, String>,
    number_words: HashMap<String, u32>,
    verbs: Vec<VerbDefinition>,
    nouns: Vec<NounDefinition>,
    adjectives: Vec<AdjectiveDefinition>,
    prepositions: Vec<String>,
    adverbs: Vec<String>,
    scopal_adverbs: Vec<String>,
    temporal_adverbs: Vec<String>,
    #[serde(default)]
    particles: Vec<String>,
    #[serde(default)]
    phrasal_verbs: HashMap<String, PhrasalVerbEntry>,
    not_adverbs: Vec<String>,
    noun_patterns: Vec<String>,
    disambiguation_not_verbs: Vec<String>,
    morphology: Morphology,
    #[serde(default)]
    units: HashMap<String, String>,
    #[serde(default)]
    multi_word_expressions: Vec<MweEntry>,
    #[serde(default)]
    ontology: Option<OntologyData>,
    #[serde(default)]
    axioms: Option<AxiomData>,
    #[serde(default)]
    morphological_rules: Vec<MorphologicalRule>,
}

#[derive(Deserialize)]
struct PronounEntry {
    word: String,
    gender: String,
    number: String,
    case: String,
}

#[derive(Deserialize)]
struct VerbDefinition {
    lemma: String,
    class: String,
    #[serde(default)]
    forms: Option<VerbForms>,
    #[serde(default)]
    regular: bool,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    antonyms: Vec<String>,
}

#[derive(Deserialize, Default)]
struct VerbForms {
    #[serde(default)]
    present3s: Option<String>,
    #[serde(default)]
    past: Option<String>,
    #[serde(default)]
    participle: Option<String>,
    #[serde(default)]
    gerund: Option<String>,
}

#[derive(Deserialize)]
struct NounDefinition {
    lemma: String,
    #[serde(default)]
    forms: Option<NounForms>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    derivation: Option<NounDerivation>,
}

#[derive(Deserialize, Default)]
struct NounDerivation {
    root: String,
    pos: String,
    relation: String,
}

#[derive(Deserialize)]
struct MorphologicalRule {
    suffix: String,
    base_pos: String,
    relation: String,
}

#[derive(Deserialize, Default)]
struct NounForms {
    #[serde(default)]
    plural: Option<String>,
}

#[derive(Deserialize)]
struct AdjectiveDefinition {
    lemma: String,
    #[serde(default)]
    regular: bool,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct Morphology {
    needs_e_ing: Vec<String>,
    needs_e_ed: Vec<String>,
    stemming_exceptions: Vec<String>,
}

#[derive(Deserialize)]
struct MweEntry {
    pattern: Vec<String>,
    lemma: String,
    pos: String,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct PhrasalVerbEntry {
    lemma: String,
    class: String,
}

#[derive(Deserialize, Default)]
struct OntologyData {
    #[serde(default)]
    part_whole: Vec<PartWholeEntry>,
    #[serde(default)]
    predicate_sorts: HashMap<String, String>,
}

#[derive(Deserialize)]
struct PartWholeEntry {
    whole: String,
    parts: Vec<String>,
}

#[derive(Deserialize, Default)]
struct AxiomData {
    #[serde(default)]
    nouns: HashMap<String, NounAxiom>,
    #[serde(default)]
    adjectives: HashMap<String, AdjectiveAxiom>,
    #[serde(default)]
    verbs: HashMap<String, VerbAxiom>,
}

#[derive(Deserialize, Default)]
struct NounAxiom {
    #[serde(default)]
    entails: Vec<String>,
    #[serde(default)]
    hypernyms: Vec<String>,
}

#[derive(Deserialize)]
struct AdjectiveAxiom {
    #[serde(rename = "type")]
    axiom_type: String,
}

#[derive(Deserialize, Default)]
struct VerbAxiom {
    #[serde(default)]
    entails: Option<String>,
    #[serde(default)]
    manner: Vec<String>,
}

// Intermediate representation for irregular verb entries
struct IrregularVerbEntry {
    word: String,
    lemma: String,
    time: String,
    aspect: String,
    class: String,
}

// Full verb database entry with features
struct VerbDbEntry {
    word: String,
    lemma: String,
    time: String,
    aspect: String,
    class: String,
    features: Vec<String>,
}

// Noun database entry with features
struct NounDbEntry {
    word: String,
    lemma: String,
    number: String,
    features: Vec<String>,
}

// Adjective database entry with features
struct AdjectiveDbEntry {
    word: String,
    lemma: String,
    features: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════
// Main Build Function
// ═══════════════════════════════════════════════════════════════════

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let json_path = Path::new(&manifest_dir).join("assets/lexicon.json");

    println!("cargo:rerun-if-changed=assets/lexicon.json");

    let json_content = fs::read_to_string(&json_path)
        .unwrap_or_else(|_| panic!("Failed to read {}", json_path.display()));

    let data: RefactoredLexiconData = serde_json::from_str(&json_content)
        .unwrap_or_else(|e| panic!("Failed to parse refactored_lexicon.json: {}", e));

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("lexicon_data.rs");
    let mut file = fs::File::create(&dest_path).unwrap();

    // Generate unchanged lookup functions
    generate_lookup_keyword(&mut file, &data.keywords);
    generate_lookup_pronoun(&mut file, &data.pronouns);
    generate_lookup_article(&mut file, &data.articles);
    generate_lookup_auxiliary(&mut file, &data.auxiliaries);
    generate_lookup_presup_trigger(&mut file, &data.presupposition_triggers);
    generate_word_to_number(&mut file, &data.number_words);

    // Expand verbs into irregular verb entries and generate lookup
    let irregular_verbs = expand_verbs_to_entries(&data.verbs);
    generate_lookup_irregular_verb(&mut file, &irregular_verbs);

    // Generate singularize from noun forms
    let irregular_plurals = derive_irregular_plurals(&data.nouns);
    generate_singularize(&mut file, &irregular_plurals);

    // Generate closed class checks
    generate_is_check(&mut file, "is_preposition", &data.prepositions);
    generate_is_check(&mut file, "is_noun_pattern", &data.noun_patterns);
    generate_is_check(&mut file, "is_scopal_adverb", &data.scopal_adverbs);
    generate_is_check(&mut file, "is_temporal_adverb", &data.temporal_adverbs);
    generate_is_check(&mut file, "is_particle", &data.particles);
    generate_is_check(&mut file, "is_adverb", &data.adverbs);
    generate_is_check(&mut file, "is_not_adverb", &data.not_adverbs);
    generate_is_check(&mut file, "is_disambiguation_not_verb", &data.disambiguation_not_verbs);
    generate_is_check(&mut file, "needs_e_ing", &data.morphology.needs_e_ing);
    generate_is_check(&mut file, "needs_e_ed", &data.morphology.needs_e_ed);
    generate_is_check(&mut file, "is_stemming_exception", &data.morphology.stemming_exceptions);
    generate_is_check(
        &mut file,
        "is_irregular_plural",
        &irregular_plurals.keys().cloned().collect::<Vec<_>>(),
    );

    // Derive behavioral lists from verb features
    let (
        ditransitive_verbs,
        subject_control_verbs,
        object_control_verbs,
        raising_verbs,
        opaque_verbs,
        collective_verbs,
        performatives,
        mixed_verbs,
        distributive_verbs,
    ) = derive_verb_feature_lists(&data.verbs);

    generate_is_check(&mut file, "is_ditransitive_verb", &ditransitive_verbs);
    generate_is_check(&mut file, "is_subject_control_verb", &subject_control_verbs);
    generate_is_check(&mut file, "is_object_control_verb", &object_control_verbs);
    generate_is_check(&mut file, "is_raising_verb", &raising_verbs);
    generate_is_check(&mut file, "is_opaque_verb", &opaque_verbs);
    generate_is_check(&mut file, "is_collective_verb", &collective_verbs);
    generate_is_check(&mut file, "is_performative", &performatives);
    generate_is_check(&mut file, "is_mixed_verb", &mixed_verbs);
    generate_is_check(&mut file, "is_distributive_verb", &distributive_verbs);

    // Generate base verb list from all verb lemmas
    let base_verbs: Vec<String> = data.verbs.iter().map(|v| v.lemma.to_lowercase()).collect();
    generate_is_check(&mut file, "is_base_verb", &base_verbs);
    generate_is_check(&mut file, "is_base_verb_early", &base_verbs[..base_verbs.len().min(30)].to_vec());
    generate_is_check(&mut file, "is_infinitive_verb", &base_verbs);

    // Derive adjective lists from features
    let (adjectives, non_intersective, subsective, gradable, event_modifier) = derive_adjective_lists(&data.adjectives);
    generate_is_check(&mut file, "is_adjective", &adjectives);
    generate_is_check(&mut file, "is_non_intersective", &non_intersective);
    generate_is_check(&mut file, "is_subsective", &subsective);
    generate_is_check(&mut file, "is_gradable_adjective", &gradable);
    generate_is_check(&mut file, "is_event_modifier_adjective", &event_modifier);

    // Derive noun lists from features
    let (common_nouns, male_names, female_names, male_nouns, female_nouns, neuter_nouns) =
        derive_noun_lists(&data.nouns);
    generate_is_check(&mut file, "is_common_noun", &common_nouns);
    generate_is_check(&mut file, "is_male_name", &male_names);
    generate_is_check(&mut file, "is_female_name", &female_names);
    generate_is_check(&mut file, "is_male_noun", &male_nouns);
    generate_is_check(&mut file, "is_female_noun", &female_nouns);
    generate_is_check(&mut file, "is_neuter_noun", &neuter_nouns);

    // Generate verb class lookup from verb definitions
    let (state_verbs, activity_verbs, accomplishment_verbs, achievement_verbs, semelfactive_verbs) =
        derive_verb_class_lists(&data.verbs);
    generate_lookup_verb_class(
        &mut file,
        &state_verbs,
        &activity_verbs,
        &accomplishment_verbs,
        &achievement_verbs,
        &semelfactive_verbs,
    );

    // Generate feature-based metadata databases
    let verb_db_entries = expand_verbs_to_db_entries(&data.verbs);
    generate_lookup_verb_db(&mut file, &verb_db_entries);

    let noun_db_entries = expand_nouns_to_db_entries(&data.nouns);
    generate_lookup_noun_db(&mut file, &noun_db_entries);

    let adjective_db_entries = expand_adjectives_to_db_entries(&data.adjectives);
    generate_lookup_adjective_db(&mut file, &adjective_db_entries);

    // Generate unit dimension lookup for degree semantics
    generate_lookup_unit_dimension(&mut file, &data.units);

    // Generate phrasal verb lookup for particle movement
    generate_lookup_phrasal_verb(&mut file, &data.phrasal_verbs);

    // Generate sort lookup for semantic type system
    generate_lookup_sort(&mut file, &data.nouns);

    // Generate MWE trie initialization
    let mwe_path = Path::new(&out_dir).join("mwe_data.rs");
    let mut mwe_file = fs::File::create(&mwe_path).unwrap();
    generate_mwe_trie_init(&mut mwe_file, &data.multi_word_expressions);

    // Generate ontology lookup functions
    let ontology_path = Path::new(&out_dir).join("ontology_data.rs");
    let mut ontology_file = fs::File::create(&ontology_path).unwrap();
    generate_ontology_data(&mut ontology_file, &data.ontology);

    // Generate axiom lookup functions
    let axiom_path = Path::new(&out_dir).join("axiom_data.rs");
    let mut axiom_file = fs::File::create(&axiom_path).unwrap();
    generate_axiom_data(&mut axiom_file, &data.axioms);

    // Generate canonical mapping lookup for synonyms/antonyms
    generate_canonical_mapping(&mut file, &data.verbs);

    // Generate morphological rules for derivational morphology
    generate_morphological_rules(&mut file, &data.morphological_rules);

    // Generate noun derivation lookups (replaces agentive_nouns)
    generate_lookup_noun_derivation(&mut file, &data.nouns);
}

// ═══════════════════════════════════════════════════════════════════
// Verb Form Expansion
// ═══════════════════════════════════════════════════════════════════

fn expand_verbs_to_entries(verbs: &[VerbDefinition]) -> Vec<IrregularVerbEntry> {
    let mut entries = Vec::new();

    for verb in verbs {
        let lemma = &verb.lemma;
        let class = &verb.class;
        let lower_lemma = lemma.to_lowercase();

        // Always add the base form (present)
        entries.push(IrregularVerbEntry {
            word: lower_lemma.clone(),
            lemma: lemma.clone(),
            time: "Present".to_string(),
            aspect: "Simple".to_string(),
            class: class.clone(),
        });

        if let Some(forms) = &verb.forms {
            // Irregular verb with explicit forms
            if let Some(present3s) = &forms.present3s {
                entries.push(IrregularVerbEntry {
                    word: present3s.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Present".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                });
            }

            if let Some(past) = &forms.past {
                entries.push(IrregularVerbEntry {
                    word: past.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Past".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                });
            }

            if let Some(participle) = &forms.participle {
                // Participle is past aspect simple (for "has eaten", "was broken")
                entries.push(IrregularVerbEntry {
                    word: participle.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Past".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                });
            }

            if let Some(gerund) = &forms.gerund {
                entries.push(IrregularVerbEntry {
                    word: gerund.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "None".to_string(),
                    aspect: "Progressive".to_string(),
                    class: class.clone(),
                });
            }
        }
        // Note: Regular verbs (regular: true without forms) are handled by
        // Lexicon::lookup_verb's morphological rules, not the irregular table
    }

    entries
}

// ═══════════════════════════════════════════════════════════════════
// Feature Derivation Functions
// ═══════════════════════════════════════════════════════════════════

fn derive_verb_feature_lists(
    verbs: &[VerbDefinition],
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let mut ditransitive = Vec::new();
    let mut subject_control = Vec::new();
    let mut object_control = Vec::new();
    let mut raising = Vec::new();
    let mut opaque = Vec::new();
    let mut collective = Vec::new();
    let mut performative = Vec::new();
    let mut mixed = Vec::new();
    let mut distributive = Vec::new();

    for verb in verbs {
        let lower = verb.lemma.to_lowercase();
        for feature in &verb.features {
            match feature.as_str() {
                "Ditransitive" => ditransitive.push(lower.clone()),
                "SubjectControl" => subject_control.push(lower.clone()),
                "ObjectControl" => object_control.push(lower.clone()),
                "Raising" => raising.push(lower.clone()),
                "Opaque" => {
                    // Include base form and conjugated forms for opaque verb checks
                    opaque.push(lower.clone());
                    opaque.push(format!("{}s", lower)); // third person singular
                    opaque.push(format!("{}ed", lower)); // past tense (regular)
                    // Also include irregular forms if present
                    if let Some(forms) = &verb.forms {
                        if let Some(past) = &forms.past {
                            opaque.push(past.to_lowercase());
                        }
                        if let Some(participle) = &forms.participle {
                            opaque.push(participle.to_lowercase());
                        }
                    }
                }
                "Collective" => collective.push(lower.clone()),
                "Performative" => performative.push(lower.clone()),
                "Mixed" => mixed.push(lower.clone()),
                "Distributive" => distributive.push(lower.clone()),
                _ => {}
            }
        }
    }

    (
        ditransitive,
        subject_control,
        object_control,
        raising,
        opaque,
        collective,
        performative,
        mixed,
        distributive,
    )
}

fn derive_verb_class_lists(
    verbs: &[VerbDefinition],
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut state = Vec::new();
    let mut activity = Vec::new();
    let mut accomplishment = Vec::new();
    let mut achievement = Vec::new();
    let mut semelfactive = Vec::new();

    for verb in verbs {
        let lower = verb.lemma.to_lowercase();
        match verb.class.as_str() {
            "State" => state.push(lower),
            "Activity" => activity.push(lower),
            "Accomplishment" => accomplishment.push(lower),
            "Achievement" => achievement.push(lower),
            "Semelfactive" => semelfactive.push(lower),
            _ => activity.push(lower),
        }
    }

    (state, activity, accomplishment, achievement, semelfactive)
}

fn derive_adjective_lists(
    adjectives: &[AdjectiveDefinition],
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut all_adj = Vec::new();
    let mut non_intersective = Vec::new();
    let mut subsective = Vec::new();
    let mut gradable = Vec::new();
    let mut event_modifier = Vec::new();

    for adj in adjectives {
        let lower = adj.lemma.to_lowercase();
        all_adj.push(lower.clone());

        for feature in &adj.features {
            match feature.as_str() {
                "NonIntersective" => non_intersective.push(lower.clone()),
                "Subsective" => subsective.push(lower.clone()),
                "Gradable" => gradable.push(lower.clone()),
                "EventModifier" => event_modifier.push(lower.clone()),
                _ => {}
            }
        }
    }

    (all_adj, non_intersective, subsective, gradable, event_modifier)
}

fn derive_noun_lists(
    nouns: &[NounDefinition],
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut common_nouns = Vec::new();
    let mut male_names = Vec::new();
    let mut female_names = Vec::new();
    let mut male_nouns = Vec::new();
    let mut female_nouns = Vec::new();
    let mut neuter_nouns = Vec::new();

    for noun in nouns {
        let lower = noun.lemma.to_lowercase();
        let is_proper = noun.features.iter().any(|f| f == "Proper");
        let is_masculine = noun.features.iter().any(|f| f == "Masculine");
        let is_feminine = noun.features.iter().any(|f| f == "Feminine");
        let is_neuter = noun.features.iter().any(|f| f == "Neuter");

        if is_proper {
            if is_masculine {
                male_names.push(lower.clone());
            }
            if is_feminine {
                female_names.push(lower.clone());
            }
        } else {
            common_nouns.push(lower.clone());
            if is_masculine {
                male_nouns.push(lower.clone());
            }
            if is_feminine {
                female_nouns.push(lower.clone());
            }
            if is_neuter {
                neuter_nouns.push(lower.clone());
            }
        }
    }

    (common_nouns, male_names, female_names, male_nouns, female_nouns, neuter_nouns)
}

fn derive_irregular_plurals(nouns: &[NounDefinition]) -> HashMap<String, String> {
    let mut plurals = HashMap::new();
    for noun in nouns {
        if let Some(forms) = &noun.forms {
            if let Some(plural) = &forms.plural {
                plurals.insert(plural.to_lowercase(), noun.lemma.clone());
            }
        }
    }
    plurals
}

// ═══════════════════════════════════════════════════════════════════
// Code Generation Functions
// ═══════════════════════════════════════════════════════════════════

fn generate_lookup_keyword(file: &mut fs::File, keywords: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn lookup_keyword(s: &str) -> Option<crate::token::TokenType> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (word, token_type) in keywords {
        let token_expr = match token_type.as_str() {
            "All" => "crate::token::TokenType::All",
            "No" => "crate::token::TokenType::No",
            "Some" => "crate::token::TokenType::Some",
            "Any" => "crate::token::TokenType::Any",
            "Both" => "crate::token::TokenType::Both",
            "Most" => "crate::token::TokenType::Most",
            "Few" => "crate::token::TokenType::Few",
            "Many" => "crate::token::TokenType::Many",
            "And" => "crate::token::TokenType::And",
            "Or" => "crate::token::TokenType::Or",
            "If" => "crate::token::TokenType::If",
            "Then" => "crate::token::TokenType::Then",
            "Not" => "crate::token::TokenType::Not",
            "Is" => "crate::token::TokenType::Is",
            "Are" => "crate::token::TokenType::Are",
            "Was" => "crate::token::TokenType::Was",
            "Were" => "crate::token::TokenType::Were",
            "That" => "crate::token::TokenType::That",
            "Who" => "crate::token::TokenType::Who",
            "What" => "crate::token::TokenType::What",
            "Where" => "crate::token::TokenType::Where",
            "When" => "crate::token::TokenType::When",
            "Why" => "crate::token::TokenType::Why",
            "Does" => "crate::token::TokenType::Does",
            "Do" => "crate::token::TokenType::Do",
            "Must" => "crate::token::TokenType::Must",
            "Shall" => "crate::token::TokenType::Shall",
            "Should" => "crate::token::TokenType::Should",
            "Can" => "crate::token::TokenType::Can",
            "May" => "crate::token::TokenType::May",
            "Cannot" => "crate::token::TokenType::Cannot",
            "Would" => "crate::token::TokenType::Would",
            "Could" => "crate::token::TokenType::Could",
            "Might" => "crate::token::TokenType::Might",
            "Had" => "crate::token::TokenType::Had",
            "Than" => "crate::token::TokenType::Than",
            "Reflexive" => "crate::token::TokenType::Reflexive",
            "Because" => "crate::token::TokenType::Because",
            "Anything" => "crate::token::TokenType::Anything",
            "Anyone" => "crate::token::TokenType::Anyone",
            "Nothing" => "crate::token::TokenType::Nothing",
            "Nobody" => "crate::token::TokenType::Nobody",
            "Nowhere" => "crate::token::TokenType::Nowhere",
            "Ever" => "crate::token::TokenType::Ever",
            "Never" => "crate::token::TokenType::Never",
            "Repeat" => "crate::token::TokenType::Repeat",
            "For" => "crate::token::TokenType::For",
            "In" => "crate::token::TokenType::In",
            "From" => "crate::token::TokenType::From",
            "Respectively" => "crate::token::TokenType::Respectively",
            _ => continue,
        };
        writeln!(file, "        \"{}\" => Some({}),", word, token_expr).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn format_pronoun_token(p: &PronounEntry) -> String {
    let gender = match p.gender.as_str() {
        "Male" => "crate::drs::Gender::Male",
        "Female" => "crate::drs::Gender::Female",
        "Neuter" => "crate::drs::Gender::Neuter",
        _ => "crate::drs::Gender::Unknown",
    };
    let number = match p.number.as_str() {
        "Singular" => "crate::drs::Number::Singular",
        _ => "crate::drs::Number::Plural",
    };
    let case = match p.case.as_str() {
        "Subject" => "crate::drs::Case::Subject",
        "Possessive" => "crate::drs::Case::Possessive",
        _ => "crate::drs::Case::Object",
    };
    format!(
        "crate::token::TokenType::Pronoun {{ gender: {}, number: {}, case: {} }}",
        gender, number, case
    )
}

fn generate_lookup_pronoun(file: &mut fs::File, pronouns: &[PronounEntry]) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_pronoun(s: &str) -> Option<crate::token::TokenType> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    let mut map: BTreeMap<String, Vec<&PronounEntry>> = BTreeMap::new();
    for p in pronouns {
        map.entry(p.word.to_lowercase()).or_default().push(p);
    }

    for (word, entries) in map {
        if entries.len() == 1 {
            let code = format_pronoun_token(entries[0]);
            writeln!(file, "        \"{}\" => Some({}),", word, code).unwrap();
        } else {
            let primary_code = format_pronoun_token(entries[0]);
            let mut alt_codes = Vec::new();
            for p in &entries[1..] {
                alt_codes.push(format_pronoun_token(p));
            }
            let alts_str = alt_codes.join(", ");

            writeln!(
                file,
                "        \"{}\" => Some(crate::token::TokenType::Ambiguous {{ primary: Box::new({}), alternatives: vec![{}] }}),",
                word, primary_code, alts_str
            ).unwrap();
        }
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_article(file: &mut fs::File, articles: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn lookup_article(s: &str) -> Option<crate::lexicon::Definiteness> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (word, def) in articles {
        let def_expr = match def.as_str() {
            "Definite" => "crate::lexicon::Definiteness::Definite",
            "Proximal" => "crate::lexicon::Definiteness::Proximal",
            "Distal" => "crate::lexicon::Definiteness::Distal",
            _ => "crate::lexicon::Definiteness::Indefinite",
        };
        writeln!(file, "        \"{}\" => Some({}),", word, def_expr).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_auxiliary(file: &mut fs::File, auxiliaries: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn lookup_auxiliary(s: &str) -> Option<crate::lexicon::Time> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (word, time) in auxiliaries {
        let time_expr = match time.as_str() {
            "Future" => "crate::lexicon::Time::Future",
            "Past" => "crate::lexicon::Time::Past",
            "Present" => "crate::lexicon::Time::Present",
            _ => "crate::lexicon::Time::None",
        };
        writeln!(file, "        \"{}\" => Some({}),", word, time_expr).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_irregular_verb(file: &mut fs::File, verbs: &[IrregularVerbEntry]) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_irregular_verb(s: &str) -> Option<crate::lexicon::VerbEntry> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    let mut seen: BTreeMap<String, &IrregularVerbEntry> = BTreeMap::new();
    for v in verbs {
        seen.entry(v.word.to_lowercase()).or_insert(v);
    }

    for (word, v) in seen {
        let time_expr = match v.time.as_str() {
            "Past" => "crate::lexicon::Time::Past",
            "Present" => "crate::lexicon::Time::Present",
            "Future" => "crate::lexicon::Time::Future",
            _ => "crate::lexicon::Time::None",
        };
        let aspect_expr = match v.aspect.as_str() {
            "Progressive" => "crate::lexicon::Aspect::Progressive",
            "Perfect" => "crate::lexicon::Aspect::Perfect",
            _ => "crate::lexicon::Aspect::Simple",
        };
        let class_expr = match v.class.as_str() {
            "State" => "crate::lexicon::VerbClass::State",
            "Activity" => "crate::lexicon::VerbClass::Activity",
            "Accomplishment" => "crate::lexicon::VerbClass::Accomplishment",
            "Achievement" => "crate::lexicon::VerbClass::Achievement",
            "Semelfactive" => "crate::lexicon::VerbClass::Semelfactive",
            _ => "crate::lexicon::VerbClass::Activity",
        };

        writeln!(
            file,
            "        \"{}\" => Some(crate::lexicon::VerbEntry {{ lemma: \"{}\".to_string(), time: {}, aspect: {}, class: {} }}),",
            word, v.lemma, time_expr, aspect_expr, class_expr
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_presup_trigger(file: &mut fs::File, triggers: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn lookup_presup_trigger(s: &str) -> Option<crate::token::PresupKind> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (word, kind) in triggers {
        let kind_expr = match kind.as_str() {
            "Stop" => "crate::token::PresupKind::Stop",
            "Start" => "crate::token::PresupKind::Start",
            "Regret" => "crate::token::PresupKind::Regret",
            "Continue" => "crate::token::PresupKind::Continue",
            "Realize" => "crate::token::PresupKind::Realize",
            "Know" => "crate::token::PresupKind::Know",
            _ => continue,
        };
        writeln!(file, "        \"{}\" => Some({}),", word, kind_expr).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_singularize(file: &mut fs::File, plurals: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn singularize(s: &str) -> Option<&'static str> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (plural, singular) in plurals {
        writeln!(file, "        \"{}\" => Some(\"{}\"),", plural, singular).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_word_to_number(file: &mut fs::File, numbers: &HashMap<String, u32>) {
    writeln!(
        file,
        "pub fn word_to_number(s: &str) -> Option<u32> {{"
    )
    .unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    for (word, num) in numbers {
        writeln!(file, "        \"{}\" => Some({}),", word, num).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_is_check(file: &mut fs::File, fn_name: &str, words: &[String]) {
    use std::collections::BTreeSet;

    writeln!(file, "pub fn {}(s: &str) -> bool {{", fn_name).unwrap();
    writeln!(file, "    match s.to_lowercase().as_str() {{").unwrap();

    let unique_words: BTreeSet<String> = words.iter().map(|w| w.to_lowercase()).collect();
    for word in unique_words {
        writeln!(file, "        \"{}\" => true,", word).unwrap();
    }

    writeln!(file, "        _ => false,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_verb_class(
    file: &mut fs::File,
    state_verbs: &[String],
    activity_verbs: &[String],
    accomplishment_verbs: &[String],
    achievement_verbs: &[String],
    semelfactive_verbs: &[String],
) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_verb_class(lemma: &str) -> crate::lexicon::VerbClass {{"
    )
    .unwrap();
    writeln!(file, "    match lemma.to_lowercase().as_str() {{").unwrap();

    let mut verb_classes: BTreeMap<String, &str> = BTreeMap::new();
    for verb in state_verbs {
        verb_classes.insert(verb.to_lowercase(), "State");
    }
    for verb in activity_verbs {
        verb_classes.entry(verb.to_lowercase()).or_insert("Activity");
    }
    for verb in accomplishment_verbs {
        verb_classes.entry(verb.to_lowercase()).or_insert("Accomplishment");
    }
    for verb in achievement_verbs {
        verb_classes.entry(verb.to_lowercase()).or_insert("Achievement");
    }
    for verb in semelfactive_verbs {
        verb_classes.entry(verb.to_lowercase()).or_insert("Semelfactive");
    }

    for (verb, class) in verb_classes {
        let class_expr = match class {
            "State" => "crate::lexicon::VerbClass::State",
            "Activity" => "crate::lexicon::VerbClass::Activity",
            "Accomplishment" => "crate::lexicon::VerbClass::Accomplishment",
            "Achievement" => "crate::lexicon::VerbClass::Achievement",
            "Semelfactive" => "crate::lexicon::VerbClass::Semelfactive",
            _ => "crate::lexicon::VerbClass::Activity",
        };
        writeln!(file, "        \"{}\" => {},", verb, class_expr).unwrap();
    }

    writeln!(file, "        _ => crate::lexicon::VerbClass::Activity,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Feature-Based Database Generation
// ═══════════════════════════════════════════════════════════════════

fn expand_verbs_to_db_entries(verbs: &[VerbDefinition]) -> Vec<VerbDbEntry> {
    let mut entries = Vec::new();

    for verb in verbs {
        let lemma = &verb.lemma;
        let class = &verb.class;
        let features = verb.features.clone();
        let lower_lemma = lemma.to_lowercase();

        // Base form (present)
        entries.push(VerbDbEntry {
            word: lower_lemma.clone(),
            lemma: lemma.clone(),
            time: "Present".to_string(),
            aspect: "Simple".to_string(),
            class: class.clone(),
            features: features.clone(),
        });

        if let Some(forms) = &verb.forms {
            if let Some(present3s) = &forms.present3s {
                entries.push(VerbDbEntry {
                    word: present3s.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Present".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                    features: features.clone(),
                });
            }

            if let Some(past) = &forms.past {
                entries.push(VerbDbEntry {
                    word: past.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Past".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                    features: features.clone(),
                });
            }

            if let Some(participle) = &forms.participle {
                entries.push(VerbDbEntry {
                    word: participle.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "Past".to_string(),
                    aspect: "Simple".to_string(),
                    class: class.clone(),
                    features: features.clone(),
                });
            }

            if let Some(gerund) = &forms.gerund {
                entries.push(VerbDbEntry {
                    word: gerund.to_lowercase(),
                    lemma: lemma.clone(),
                    time: "None".to_string(),
                    aspect: "Progressive".to_string(),
                    class: class.clone(),
                    features: features.clone(),
                });
            }
        }
    }

    entries
}

fn expand_nouns_to_db_entries(nouns: &[NounDefinition]) -> Vec<NounDbEntry> {
    let mut entries = Vec::new();

    for noun in nouns {
        let lemma = &noun.lemma;
        let features = noun.features.clone();
        let lower_lemma = lemma.to_lowercase();

        // Singular form
        entries.push(NounDbEntry {
            word: lower_lemma.clone(),
            lemma: lemma.clone(),
            number: "Singular".to_string(),
            features: features.clone(),
        });

        // Plural form
        if let Some(forms) = &noun.forms {
            if let Some(plural) = &forms.plural {
                entries.push(NounDbEntry {
                    word: plural.to_lowercase(),
                    lemma: lemma.clone(),
                    number: "Plural".to_string(),
                    features: features.clone(),
                });
            }
        }
    }

    entries
}

fn expand_adjectives_to_db_entries(adjectives: &[AdjectiveDefinition]) -> Vec<AdjectiveDbEntry> {
    let mut entries = Vec::new();

    for adj in adjectives {
        let lemma = &adj.lemma;
        let features = adj.features.clone();
        let lower_lemma = lemma.to_lowercase();

        entries.push(AdjectiveDbEntry {
            word: lower_lemma,
            lemma: lemma.clone(),
            features,
        });
    }

    entries
}

fn format_features(features: &[String]) -> String {
    if features.is_empty() {
        return "&[]".to_string();
    }
    let feature_strs: Vec<String> = features
        .iter()
        .map(|f| format!("crate::lexicon::Feature::{}", f))
        .collect();
    format!("&[{}]", feature_strs.join(", "))
}

fn generate_lookup_verb_db(file: &mut fs::File, entries: &[VerbDbEntry]) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_verb_db(word: &str) -> Option<crate::lexicon::VerbMetadata> {{"
    )
    .unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    let mut seen: BTreeMap<String, &VerbDbEntry> = BTreeMap::new();
    for entry in entries {
        seen.entry(entry.word.to_lowercase()).or_insert(entry);
    }

    for (word, entry) in seen {
        let time_expr = match entry.time.as_str() {
            "Past" => "crate::lexicon::Time::Past",
            "Present" => "crate::lexicon::Time::Present",
            "Future" => "crate::lexicon::Time::Future",
            _ => "crate::lexicon::Time::None",
        };
        let aspect_expr = match entry.aspect.as_str() {
            "Progressive" => "crate::lexicon::Aspect::Progressive",
            "Perfect" => "crate::lexicon::Aspect::Perfect",
            _ => "crate::lexicon::Aspect::Simple",
        };
        let class_expr = match entry.class.as_str() {
            "State" => "crate::lexicon::VerbClass::State",
            "Activity" => "crate::lexicon::VerbClass::Activity",
            "Accomplishment" => "crate::lexicon::VerbClass::Accomplishment",
            "Achievement" => "crate::lexicon::VerbClass::Achievement",
            "Semelfactive" => "crate::lexicon::VerbClass::Semelfactive",
            _ => "crate::lexicon::VerbClass::Activity",
        };
        let features_expr = format_features(&entry.features);

        writeln!(
            file,
            "        \"{}\" => Some(crate::lexicon::VerbMetadata {{ lemma: \"{}\", class: {}, time: {}, aspect: {}, features: {} }}),",
            word, entry.lemma, class_expr, time_expr, aspect_expr, features_expr
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_noun_db(file: &mut fs::File, entries: &[NounDbEntry]) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_noun_db(word: &str) -> Option<crate::lexicon::NounMetadata> {{"
    )
    .unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    let mut seen: BTreeMap<String, &NounDbEntry> = BTreeMap::new();
    for entry in entries {
        seen.entry(entry.word.to_lowercase()).or_insert(entry);
    }

    for (word, entry) in seen {
        let number_expr = match entry.number.as_str() {
            "Plural" => "crate::lexicon::Number::Plural",
            _ => "crate::lexicon::Number::Singular",
        };
        let features_expr = format_features(&entry.features);

        writeln!(
            file,
            "        \"{}\" => Some(crate::lexicon::NounMetadata {{ lemma: \"{}\", number: {}, features: {} }}),",
            word, entry.lemma, number_expr, features_expr
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_adjective_db(file: &mut fs::File, entries: &[AdjectiveDbEntry]) {
    use std::collections::BTreeMap;

    writeln!(
        file,
        "pub fn lookup_adjective_db(word: &str) -> Option<crate::lexicon::AdjectiveMetadata> {{"
    )
    .unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    let mut seen: BTreeMap<String, &AdjectiveDbEntry> = BTreeMap::new();
    for entry in entries {
        seen.entry(entry.word.to_lowercase()).or_insert(entry);
    }

    for (word, entry) in seen {
        let features_expr = format_features(&entry.features);

        writeln!(
            file,
            "        \"{}\" => Some(crate::lexicon::AdjectiveMetadata {{ lemma: \"{}\", features: {} }}),",
            word, entry.lemma, features_expr
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_unit_dimension(file: &mut fs::File, units: &HashMap<String, String>) {
    writeln!(
        file,
        "pub fn lookup_unit_dimension(word: &str) -> Option<crate::ast::Dimension> {{"
    )
    .unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for (word, dimension) in units {
        let dim_expr = match dimension.as_str() {
            "Length" => "crate::ast::Dimension::Length",
            "Time" => "crate::ast::Dimension::Time",
            "Weight" => "crate::ast::Dimension::Weight",
            "Temperature" => "crate::ast::Dimension::Temperature",
            "Cardinality" => "crate::ast::Dimension::Cardinality",
            _ => continue,
        };
        writeln!(file, "        \"{}\" => Some({}),", word, dim_expr).unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_phrasal_verb(file: &mut fs::File, phrasal_verbs: &HashMap<String, PhrasalVerbEntry>) {
    writeln!(
        file,
        "pub fn lookup_phrasal_verb(verb: &str, particle: &str) -> Option<(&'static str, crate::lexicon::VerbClass)> {{"
    )
    .unwrap();
    writeln!(file, "    let key = format!(\"{{}}_{{}}\", verb.to_lowercase(), particle.to_lowercase());").unwrap();
    writeln!(file, "    match key.as_str() {{").unwrap();

    for (key, entry) in phrasal_verbs {
        let class_expr = match entry.class.as_str() {
            "State" => "crate::lexicon::VerbClass::State",
            "Activity" => "crate::lexicon::VerbClass::Activity",
            "Accomplishment" => "crate::lexicon::VerbClass::Accomplishment",
            "Achievement" => "crate::lexicon::VerbClass::Achievement",
            "Semelfactive" => "crate::lexicon::VerbClass::Semelfactive",
            _ => "crate::lexicon::VerbClass::Activity",
        };
        writeln!(
            file,
            "        \"{}\" => Some((\"{}\", {})),",
            key, entry.lemma, class_expr
        )
        .unwrap();
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

fn generate_lookup_sort(file: &mut fs::File, nouns: &[NounDefinition]) {
    writeln!(
        file,
        "pub fn lookup_sort(word: &str) -> Option<crate::lexicon::Sort> {{"
    )
    .unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for noun in nouns {
        if let Some(sort) = &noun.sort {
            let sort_expr = match sort.as_str() {
                "Entity" => "crate::lexicon::Sort::Entity",
                "Physical" => "crate::lexicon::Sort::Physical",
                "Animate" => "crate::lexicon::Sort::Animate",
                "Human" => "crate::lexicon::Sort::Human",
                "Plant" => "crate::lexicon::Sort::Plant",
                "Place" => "crate::lexicon::Sort::Place",
                "Time" => "crate::lexicon::Sort::Time",
                "Abstract" => "crate::lexicon::Sort::Abstract",
                "Information" => "crate::lexicon::Sort::Information",
                "Event" => "crate::lexicon::Sort::Event",
                "Celestial" => "crate::lexicon::Sort::Celestial",
                "Value" => "crate::lexicon::Sort::Value",
                "Group" => "crate::lexicon::Sort::Group",
                _ => continue,
            };
            writeln!(
                file,
                "        \"{}\" => Some({}),",
                noun.lemma.to_lowercase(),
                sort_expr
            )
            .unwrap();
        }
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}\n").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Multi-Word Expression (MWE) Generation
// ═══════════════════════════════════════════════════════════════════

fn generate_mwe_trie_init(file: &mut fs::File, mwes: &[MweEntry]) {
    writeln!(file, "/// Build the MWE trie from lexicon data.").unwrap();
    writeln!(file, "pub fn build_mwe_trie() -> MweTrie {{").unwrap();
    writeln!(file, "    let mut trie = MweTrie::default();").unwrap();

    for mwe in mwes {
        let pattern: Vec<String> = mwe
            .pattern
            .iter()
            .map(|s| format!("\"{}\"", s.to_lowercase()))
            .collect();
        let class_expr = match &mwe.class {
            Some(c) => format!("Some(crate::lexicon::VerbClass::{})", c),
            None => "None".to_string(),
        };
        writeln!(
            file,
            "    trie.insert(&[{}], MweTarget {{ lemma: \"{}\", pos: \"{}\", class: {} }});",
            pattern.join(", "),
            mwe.lemma,
            mwe.pos,
            class_expr
        )
        .unwrap();
    }

    writeln!(file, "    trie").unwrap();
    writeln!(file, "}}").unwrap();
}

fn generate_ontology_data(file: &mut fs::File, ontology: &Option<OntologyData>) {
    let default_ontology = OntologyData::default();
    let ontology = ontology.as_ref().unwrap_or(&default_ontology);

    // Build reverse mapping: part -> list of wholes
    let mut part_to_wholes: HashMap<String, Vec<String>> = HashMap::new();
    for entry in &ontology.part_whole {
        for part in &entry.parts {
            part_to_wholes
                .entry(part.to_lowercase())
                .or_default()
                .push(entry.whole.clone());
        }
    }

    // Generate get_possible_wholes function
    writeln!(file, "/// Get possible whole objects for a given part noun.").unwrap();
    writeln!(file, "pub fn get_possible_wholes(part: &str) -> &'static [&'static str] {{").unwrap();
    writeln!(file, "    match part {{").unwrap();
    for (part, wholes) in &part_to_wholes {
        let wholes_str: Vec<String> = wholes.iter().map(|w| format!("\"{}\"", w)).collect();
        writeln!(file, "        \"{}\" => &[{}],", part, wholes_str.join(", ")).unwrap();
    }
    writeln!(file, "        _ => &[],").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate get_predicate_sort function
    writeln!(file, "/// Get the required sort for a predicate (adjective or verb).").unwrap();
    writeln!(file, "pub fn get_predicate_sort(predicate: &str) -> Option<crate::lexicon::Sort> {{").unwrap();
    writeln!(file, "    match predicate {{").unwrap();
    for (predicate, sort) in &ontology.predicate_sorts {
        writeln!(file, "        \"{}\" => Some(crate::lexicon::Sort::{}),", predicate.to_lowercase(), sort).unwrap();
    }
    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Axiom (Meaning Postulate) Generation
// ═══════════════════════════════════════════════════════════════════

fn generate_axiom_data(file: &mut fs::File, axioms: &Option<AxiomData>) {
    let default_axioms = AxiomData::default();
    let axioms = axioms.as_ref().unwrap_or(&default_axioms);

    // Generate lookup_noun_entailments
    writeln!(file, "/// Get entailment predicates for a noun (e.g., bachelor -> [Unmarried, Male]).").unwrap();
    writeln!(file, "pub fn lookup_noun_entailments(noun: &str) -> &'static [&'static str] {{").unwrap();
    writeln!(file, "    match noun.to_lowercase().as_str() {{").unwrap();
    for (noun, axiom) in &axioms.nouns {
        if !axiom.entails.is_empty() {
            let entails_str: Vec<String> = axiom.entails.iter().map(|e| format!("\"{}\"", e)).collect();
            writeln!(file, "        \"{}\" => &[{}],", noun, entails_str.join(", ")).unwrap();
        }
    }
    writeln!(file, "        _ => &[],").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate lookup_noun_hypernyms
    writeln!(file, "/// Get hypernym predicates for a noun (e.g., dog -> [Animal, Mammal]).").unwrap();
    writeln!(file, "pub fn lookup_noun_hypernyms(noun: &str) -> &'static [&'static str] {{").unwrap();
    writeln!(file, "    match noun.to_lowercase().as_str() {{").unwrap();
    for (noun, axiom) in &axioms.nouns {
        if !axiom.hypernyms.is_empty() {
            let hypernyms_str: Vec<String> = axiom.hypernyms.iter().map(|h| format!("\"{}\"", h)).collect();
            writeln!(file, "        \"{}\" => &[{}],", noun, hypernyms_str.join(", ")).unwrap();
        }
    }
    writeln!(file, "        _ => &[],").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate is_privative_adjective
    writeln!(file, "/// Check if an adjective is privative (e.g., fake, counterfeit).").unwrap();
    writeln!(file, "pub fn is_privative_adjective(adj: &str) -> bool {{").unwrap();
    writeln!(file, "    match adj.to_lowercase().as_str() {{").unwrap();
    for (adj, axiom) in &axioms.adjectives {
        if axiom.axiom_type == "Privative" {
            writeln!(file, "        \"{}\" => true,", adj).unwrap();
        }
    }
    writeln!(file, "        _ => false,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate lookup_verb_entailment
    writeln!(file, "/// Get verb entailment (e.g., murder -> (Kill, [Intentional])).").unwrap();
    writeln!(file, "pub fn lookup_verb_entailment(verb: &str) -> Option<(&'static str, &'static [&'static str])> {{").unwrap();
    writeln!(file, "    match verb.to_lowercase().as_str() {{").unwrap();
    for (verb, axiom) in &axioms.verbs {
        if let Some(entails) = &axiom.entails {
            let manner_str: Vec<String> = axiom.manner.iter().map(|m| format!("\"{}\"", m)).collect();
            writeln!(file, "        \"{}\" => Some((\"{}\", &[{}])),", verb, entails, manner_str.join(", ")).unwrap();
        }
    }
    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Canonical Mapping Generation (Synonyms/Antonyms)
// ═══════════════════════════════════════════════════════════════════

fn generate_canonical_mapping(file: &mut fs::File, verbs: &[VerbDefinition]) {
    // Generate Polarity enum
    writeln!(file, "/// Polarity for canonical mapping (positive = synonym, negative = antonym).").unwrap();
    writeln!(file, "#[derive(Debug, Clone, Copy, PartialEq, Eq)]").unwrap();
    writeln!(file, "pub enum Polarity {{").unwrap();
    writeln!(file, "    Positive,").unwrap();
    writeln!(file, "    Negative,").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate CanonicalMapping struct
    writeln!(file, "/// Maps a word to its canonical form with polarity.").unwrap();
    writeln!(file, "#[derive(Debug, Clone, Copy)]").unwrap();
    writeln!(file, "pub struct CanonicalMapping {{").unwrap();
    writeln!(file, "    pub lemma: &'static str,").unwrap();
    writeln!(file, "    pub polarity: Polarity,").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate lookup_canonical function
    writeln!(file, "/// Look up canonical form for a word (synonym/antonym normalization).").unwrap();
    writeln!(file, "/// Returns the canonical lemma and polarity (Negative for antonyms).").unwrap();
    writeln!(file, "pub fn lookup_canonical(word: &str) -> Option<CanonicalMapping> {{").unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for verb in verbs {
        let lemma = &verb.lemma;

        // Map synonyms -> Positive polarity
        for syn in &verb.synonyms {
            writeln!(
                file,
                "        \"{}\" => Some(CanonicalMapping {{ lemma: \"{}\", polarity: Polarity::Positive }}),",
                syn.to_lowercase(),
                lemma
            ).unwrap();
        }

        // Map antonyms -> Negative polarity
        for ant in &verb.antonyms {
            writeln!(
                file,
                "        \"{}\" => Some(CanonicalMapping {{ lemma: \"{}\", polarity: Polarity::Negative }}),",
                ant.to_lowercase(),
                lemma
            ).unwrap();
        }
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Morphological Rules Generation
// ═══════════════════════════════════════════════════════════════════

fn generate_morphological_rules(file: &mut fs::File, rules: &[MorphologicalRule]) {
    // Generate MorphRule struct
    writeln!(file, "/// Morphological derivation rule from lexicon.").unwrap();
    writeln!(file, "#[derive(Debug, Clone, Copy)]").unwrap();
    writeln!(file, "pub struct MorphRule {{").unwrap();
    writeln!(file, "    pub suffix: &'static str,").unwrap();
    writeln!(file, "    pub base_pos: &'static str,").unwrap();
    writeln!(file, "    pub relation: &'static str,").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate get_morphological_rules function
    writeln!(file, "/// Get morphological derivation rules from lexicon.").unwrap();
    writeln!(file, "pub fn get_morphological_rules() -> &'static [MorphRule] {{").unwrap();
    writeln!(file, "    &[").unwrap();

    for rule in rules {
        writeln!(
            file,
            "        MorphRule {{ suffix: \"{}\", base_pos: \"{}\", relation: \"{}\" }},",
            rule.suffix, rule.base_pos, rule.relation
        )
        .unwrap();
    }

    writeln!(file, "    ]").unwrap();
    writeln!(file, "}}").unwrap();
}

// ═══════════════════════════════════════════════════════════════════
// Noun Derivation Lookup Generation
// ═══════════════════════════════════════════════════════════════════

fn generate_lookup_noun_derivation(file: &mut fs::File, nouns: &[NounDefinition]) {
    writeln!(file, "/// Lookup the derivation info for a noun (e.g., dancer -> (Dance, Verb, Agent))").unwrap();
    writeln!(file, "/// Returns (root, pos, relation) if the noun has a derivation.").unwrap();
    writeln!(file, "pub fn lookup_noun_derivation(word: &str) -> Option<(&'static str, &'static str, &'static str)> {{").unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for noun in nouns {
        if let Some(ref deriv) = noun.derivation {
            writeln!(
                file,
                "        \"{}\" => Some((\"{}\", \"{}\", \"{}\")),",
                noun.lemma.to_lowercase(),
                deriv.root,
                deriv.pos,
                deriv.relation
            )
            .unwrap();
        }
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Also generate a simple check for whether a noun is agentive (derived from a verb)
    writeln!(file, "/// Check if a noun is agentive (derived from a verb with Agent relation).").unwrap();
    writeln!(file, "pub fn is_agentive_noun(word: &str) -> bool {{").unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for noun in nouns {
        if let Some(ref deriv) = noun.derivation {
            if deriv.pos == "Verb" && deriv.relation == "Agent" {
                writeln!(file, "        \"{}\" => true,", noun.lemma.to_lowercase()).unwrap();
            }
        }
    }

    writeln!(file, "        _ => false,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
    writeln!(file).unwrap();

    // Generate lookup_agentive_noun for backward compatibility
    writeln!(file, "/// Lookup the base verb for an agentive noun (e.g., dancer -> Dance).").unwrap();
    writeln!(file, "/// This is for backward compatibility with existing code.").unwrap();
    writeln!(file, "pub fn lookup_agentive_noun(word: &str) -> Option<&'static str> {{").unwrap();
    writeln!(file, "    match word.to_lowercase().as_str() {{").unwrap();

    for noun in nouns {
        if let Some(ref deriv) = noun.derivation {
            if deriv.pos == "Verb" && deriv.relation == "Agent" {
                writeln!(file, "        \"{}\" => Some(\"{}\"),", noun.lemma.to_lowercase(), deriv.root).unwrap();
            }
        }
    }

    writeln!(file, "        _ => None,").unwrap();
    writeln!(file, "    }}").unwrap();
    writeln!(file, "}}").unwrap();
}
