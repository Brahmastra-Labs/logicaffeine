//! Lexicon type definitions
//!
//! Core types used by the generated lexicon lookup functions.

/// Article definiteness for noun phrases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Definiteness {
    /// The referent is uniquely identifiable ("the").
    Definite,
    /// The referent is not uniquely identifiable ("a", "an").
    Indefinite,
    /// The referent is near the speaker ("this", "these").
    Proximal,
    /// The referent is far from the speaker ("that", "those").
    Distal,
}

/// Temporal reference for verb tense.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Time {
    /// Event occurred before speech time.
    Past,
    /// Event overlaps with speech time.
    Present,
    /// Event occurs after speech time.
    Future,
    /// No temporal specification (infinitives, bare stems).
    None,
}

/// Grammatical aspect (viewpoint aspect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aspect {
    /// Event viewed as a whole, completed action.
    Simple,
    /// Event viewed as ongoing, in progress.
    Progressive,
    /// Event completed with present relevance.
    Perfect,
}

/// Vendler's Lexical Aspect Classes (Aktionsart)
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
    /// Returns true if this is a stative verb class (no change of state).
    ///
    /// States denote properties or relations that hold without change: "know", "love", "exist".
    pub fn is_stative(&self) -> bool {
        matches!(self, VerbClass::State)
    }

    /// Returns true if this verb class denotes events with duration.
    ///
    /// Durative events: States, Activities, and Accomplishments all have temporal extent.
    /// Non-durative: Achievements and Semelfactives are punctual.
    pub fn is_durative(&self) -> bool {
        matches!(
            self,
            VerbClass::State | VerbClass::Activity | VerbClass::Accomplishment
        )
    }

    /// Returns true if this verb class has an inherent endpoint (telic).
    ///
    /// Telic events: Accomplishments and Achievements reach a natural endpoint.
    /// Atelic events: States, Activities, and Semelfactives have no inherent endpoint.
    pub fn is_telic(&self) -> bool {
        matches!(self, VerbClass::Accomplishment | VerbClass::Achievement)
    }
}

/// Semantic sorts for type checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sort {
    /// Top of the hierarchy; any individual.
    Entity,
    /// Concrete, spatially located objects.
    Physical,
    /// Living beings capable of self-motion.
    Animate,
    /// Persons with intentional agency.
    Human,
    /// Non-animal living organisms.
    Plant,
    /// Locations and regions.
    Place,
    /// Temporal intervals and points.
    Time,
    /// Non-physical, conceptual entities.
    Abstract,
    /// Propositional content and data.
    Information,
    /// Occurrences and happenings.
    Event,
    /// Stars, planets, and astronomical bodies.
    Celestial,
    /// Numeric or monetary amounts.
    Value,
    /// Collections of individuals.
    Group,
}

impl Sort {
    /// Check if this sort can be used where `other` is expected.
    ///
    /// Sort compatibility follows a subsumption hierarchy:
    /// - Human ⊆ Animate ⊆ Physical ⊆ Entity
    /// - Plant ⊆ Animate ⊆ Physical ⊆ Entity
    /// - Everything ⊆ Entity
    ///
    /// For example, a Human noun can fill an Animate slot, but not vice versa.
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

/// Grammatical number for nouns and agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Number {
    /// Denotes a single individual.
    Singular,
    /// Denotes multiple individuals.
    Plural,
}

/// Grammatical gender (for pronouns and agreement).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gender {
    /// Masculine gender ("he", "him", "his").
    Male,
    /// Feminine gender ("she", "her", "hers").
    Female,
    /// Neuter gender ("it", "its").
    Neuter,
    /// Gender unspecified or indeterminate.
    Unknown,
}

/// Grammatical case (for pronouns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Case {
    /// Nominative case for subjects ("I", "he", "she").
    Subject,
    /// Accusative case for objects ("me", "him", "her").
    Object,
    /// Genitive case for possession ("my", "his", "her").
    Possessive,
}

/// Lexical polarity for canonical mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Polarity {
    /// Preserves the meaning (synonym mapping).
    Positive,
    /// Inverts the meaning (antonym mapping).
    Negative,
}

/// Lexical features that encode grammatical and semantic properties of words.
///
/// Features are assigned to lexical entries in the lexicon database and control
/// how words combine syntactically and what semantic representations they produce.
/// The feature system follows the tradition of feature-based grammar formalisms
/// like HPSG and LFG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // -------------------------------------------------------------------------
    // Verb Transitivity Features
    // -------------------------------------------------------------------------

    /// Verb requires a direct object (NP complement).
    ///
    /// Transitive verbs denote binary relations between an agent and a patient/theme.
    /// In first-order logic, they translate to two-place predicates: `Verb(x, y)`.
    ///
    /// Examples: "see", "hit", "love", "build"
    Transitive,

    /// Verb takes no object (unary predicate).
    ///
    /// Intransitive verbs denote properties of a single argument (the subject).
    /// They translate to one-place predicates: `Verb(x)`.
    ///
    /// Examples: "sleep", "arrive", "exist", "die"
    Intransitive,

    /// Verb takes two objects (direct + indirect).
    ///
    /// Ditransitive verbs denote ternary relations, typically involving transfer
    /// of possession. They translate to three-place predicates: `Verb(x, y, z)`.
    ///
    /// Examples: "give", "tell", "show", "send"
    Ditransitive,

    // -------------------------------------------------------------------------
    // Control Theory Features
    // -------------------------------------------------------------------------

    /// The subject of the matrix clause controls the PRO subject of the embedded clause.
    ///
    /// In "John promised Mary to leave", John (subject) is understood as the one leaving.
    /// Formally: promise(j, m, leave(PRO_j)) where PRO is coindexed with the subject.
    ///
    /// Examples: "promise", "try", "want", "decide"
    SubjectControl,

    /// The object of the matrix clause controls the PRO subject of the embedded clause.
    ///
    /// In "John persuaded Mary to leave", Mary (object) is understood as the one leaving.
    /// Formally: persuade(j, m, leave(PRO_m)) where PRO is coindexed with the object.
    ///
    /// Examples: "persuade", "force", "convince", "order"
    ObjectControl,

    /// Raising verb that does not assign a theta-role to its surface subject.
    ///
    /// In "John seems to be happy", "John" originates in the embedded clause and
    /// raises to matrix subject position. No control relation; subject is shared.
    /// Contrast with control: raising allows expletive subjects ("It seems that...").
    ///
    /// Examples: "seem", "appear", "happen", "tend"
    Raising,

    // -------------------------------------------------------------------------
    // Semantic Features
    // -------------------------------------------------------------------------

    /// Creates an opaque (intensional) context blocking substitution of co-referential terms.
    ///
    /// In opaque contexts, Leibniz's Law fails: even if a=b, P(a) does not entail P(b).
    /// "John believes Clark Kent is weak" does not entail "John believes Superman is weak"
    /// even if Clark Kent = Superman. Requires possible-worlds semantics.
    ///
    /// Examples: "believe", "think", "want", "seek"
    Opaque,

    /// Presupposes the truth of its complement clause.
    ///
    /// Factive verbs entail the truth of their embedded proposition regardless of
    /// the matrix clause's truth value. "John regrets that it rained" presupposes
    /// that it rained, even under negation: "John doesn't regret that it rained."
    ///
    /// Examples: "know", "regret", "realize", "discover"
    Factive,

    /// Uttering the verb constitutes performing the action it describes.
    ///
    /// Performative verbs, when uttered in first person present, do not describe
    /// an action but perform it. "I promise to come" is itself the act of promising.
    /// Austin's speech act theory: saying is doing.
    ///
    /// Examples: "promise", "declare", "pronounce", "bet"
    Performative,

    /// Requires a plural or group subject; describes collective action.
    ///
    /// Collective predicates cannot distribute over atomic individuals.
    /// "The students gathered" is true of the group, not of each student individually.
    /// Contrast with distributive: "gathered" vs "slept".
    ///
    /// Examples: "gather", "meet", "disperse", "surround"
    Collective,

    /// Can be interpreted either collectively or distributively.
    ///
    /// Mixed predicates are ambiguous between collective and distributive readings.
    /// "The students lifted the piano" can mean they lifted it together (collective)
    /// or each lifted a piano (distributive). Context disambiguates.
    ///
    /// Examples: "lift", "carry", "build", "write"
    Mixed,

    /// Distributes over atomic individuals in a plurality.
    ///
    /// Distributive predicates apply to each member of a plural subject individually.
    /// "The students slept" entails that each student slept. Formally: ∀x(student(x) → slept(x)).
    ///
    /// Examples: "sleep", "smile", "breathe", "think"
    Distributive,

    /// Impersonal verb describing meteorological phenomena; takes expletive subject.
    ///
    /// Weather verbs have no semantic subject; "it" in "it rains" is a dummy expletive.
    /// In formal semantics, they are zero-place predicates or predicates of times/events.
    ///
    /// Examples: "rain", "snow", "thunder", "drizzle"
    Weather,

    /// Intransitive verb whose subject is a theme/patient, not an agent.
    ///
    /// Unaccusative verbs have an underlying object that surfaces as subject.
    /// Evidence: auxiliary selection in Italian/German, participle agreement.
    /// "The ice melted" - the ice undergoes melting, doesn't cause it.
    ///
    /// Examples: "arrive", "fall", "melt", "appear"
    Unaccusative,

    /// Takes a proposition and evaluates it relative to possible worlds.
    ///
    /// Intensional predicates don't just operate on truth values but on intensions
    /// (functions from worlds to extensions). Required for modal and attitude reports.
    /// "John believes it might rain" involves multiple world quantification.
    ///
    /// Examples: "believe", "know", "hope", "doubt"
    IntensionalPredicate,

    // -------------------------------------------------------------------------
    // Noun Features
    // -------------------------------------------------------------------------

    /// Noun can be counted; takes singular/plural marking and numerals directly.
    ///
    /// Count nouns denote atomic, individuated entities. They combine with numerals
    /// and indefinite articles: "three cats", "a dog". Semantically, they have
    /// natural atomic minimal parts.
    ///
    /// Examples: "cat", "idea", "student", "book"
    Count,

    /// Noun denotes stuff without natural units; requires measure phrases for counting.
    ///
    /// Mass nouns are cumulative and divisive: any part of water is water, and
    /// water plus water is water. Cannot directly combine with numerals;
    /// require classifiers: "three glasses of water", not "three waters".
    ///
    /// Examples: "water", "rice", "information", "furniture"
    Mass,

    /// Noun is a proper name denoting a specific individual.
    ///
    /// Proper nouns are rigid designators that refer to the same individual in
    /// all possible worlds. They typically lack articles and don't take plural
    /// marking. Semantically, they denote individuals directly, not sets.
    ///
    /// Examples: "Socrates", "Paris", "Microsoft", "Monday"
    Proper,

    // -------------------------------------------------------------------------
    // Gender Features
    // -------------------------------------------------------------------------

    /// Grammatically masculine; triggers masculine agreement on dependents.
    ///
    /// In languages with grammatical gender, masculine nouns control agreement
    /// on articles, adjectives, and pronouns. In English, primarily affects
    /// pronoun selection for animate referents.
    ///
    /// Examples: "man", "king", "actor", "waiter"
    Masculine,

    /// Grammatically feminine; triggers feminine agreement on dependents.
    ///
    /// Feminine nouns control feminine agreement patterns. In English, primarily
    /// relevant for pronoun selection with human referents.
    ///
    /// Examples: "woman", "queen", "actress", "waitress"
    Feminine,

    /// Grammatically neuter; triggers neuter agreement on dependents.
    ///
    /// Neuter is the default for inanimate objects in English. Used for entities
    /// where natural gender is absent or unknown. "It" is the neuter pronoun.
    ///
    /// Examples: "table", "rock", "system", "idea"
    Neuter,

    // -------------------------------------------------------------------------
    // Animacy Features
    // -------------------------------------------------------------------------

    /// Denotes an entity capable of self-initiated action or sentience.
    ///
    /// Animacy is a semantic feature affecting argument realization. Animate
    /// entities can be agents, experiencers, recipients. Affects pronoun choice
    /// ("who" vs "what") and relative clause formation.
    ///
    /// Examples: "dog", "person", "bird", "robot" (ambiguous)
    Animate,

    /// Denotes a non-sentient entity incapable of self-initiated action.
    ///
    /// Inanimate entities typically serve as themes, patients, or instruments.
    /// Cannot be agents in the semantic sense. "What" rather than "who".
    ///
    /// Examples: "rock", "table", "water", "idea"
    Inanimate,

    // -------------------------------------------------------------------------
    // Adjective Features
    // -------------------------------------------------------------------------

    /// Adjective meaning combines by set intersection with noun meaning.
    ///
    /// For intersective adjectives, "A N" denotes things that are both A and N.
    /// "Red ball" means {x : red(x) ∧ ball(x)}. The adjective has a context-independent
    /// extension that intersects with the noun's extension.
    ///
    /// Examples: "red", "round", "wooden", "French"
    Intersective,

    /// Adjective meaning cannot be computed by simple intersection.
    ///
    /// Non-intersective adjectives require the noun to determine their extension.
    /// "Fake gun" is not a gun at all, so fake(x) ∧ gun(x) gives wrong results.
    /// Includes privative ("fake", "former") and modal ("alleged", "potential").
    ///
    /// Examples: "fake", "alleged", "former", "potential"
    NonIntersective,

    /// Adjective picks out a subset of the noun denotation relative to a comparison class.
    ///
    /// Subsective adjectives entail the noun: a "skillful surgeon" is a surgeon.
    /// But "skillful" is relative: skillful for a surgeon, not skillful absolutely.
    /// "Small elephant" is large for an animal but small for an elephant.
    ///
    /// Examples: "skillful", "good", "large", "small"
    Subsective,

    /// Adjective has a degree argument and supports comparison morphology.
    ///
    /// Gradable adjectives place entities on a scale with a contextual standard.
    /// "Tall" means exceeding some contextual standard of height. Supports
    /// comparatives ("taller"), superlatives ("tallest"), and degree modification.
    ///
    /// Examples: "tall", "expensive", "heavy", "smart"
    Gradable,

    /// Adjective that modifies the event denoted by the verb, not the noun.
    ///
    /// Event-modifying adjectives (when used adverbially) characterize manner or
    /// other event properties. "Careful surgeon" suggests careful in operating,
    /// not careful as a person. Related to adverb formation.
    ///
    /// Examples: "careful", "slow", "quick", "deliberate"
    EventModifier,
}

impl Feature {
    /// Parses a feature name from a string.
    ///
    /// Returns `Some(Feature)` if the string matches a known feature name (case-sensitive),
    /// or `None` if unrecognized.
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
            "Mixed" => Some(Feature::Mixed),
            "Distributive" => Some(Feature::Distributive),
            "Weather" => Some(Feature::Weather),
            "Unaccusative" => Some(Feature::Unaccusative),
            "IntensionalPredicate" => Some(Feature::IntensionalPredicate),
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

/// Verb entry returned from irregular verb lookup.
///
/// This owned struct is returned when looking up inflected verb forms
/// (e.g., "ran" → run, "went" → go). Contains the resolved morphological
/// information needed for semantic processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbEntry {
    /// The dictionary form (infinitive) of the verb.
    /// Example: "run" for input "ran", "go" for input "went".
    pub lemma: String,

    /// The temporal reference encoded by the inflection.
    /// Example: Past for "ran", Present for "runs".
    pub time: Time,

    /// The grammatical aspect of the inflected form.
    /// Example: Progressive for "running", Perfect for "run" (in "has run").
    pub aspect: Aspect,

    /// The Vendler aspectual class (Aktionsart) of the verb.
    /// Determines compatibility with temporal adverbials and aspect markers.
    pub class: VerbClass,
}

/// Static verb metadata from the lexicon database.
///
/// This borrowed struct provides zero-copy access to verb information
/// stored in the generated lexicon. Used for verbs looked up by lemma
/// rather than inflected form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbMetadata {
    /// The dictionary form (infinitive) of the verb.
    pub lemma: &'static str,

    /// The Vendler aspectual class determining temporal behavior.
    pub class: VerbClass,

    /// The default temporal reference (usually [`Time::None`] for infinitives).
    pub time: Time,

    /// The default grammatical aspect (usually [`Aspect::Simple`]).
    pub aspect: Aspect,

    /// Lexical features controlling syntax and semantics.
    /// See [`Feature`] for the full list of possible features.
    pub features: &'static [Feature],
}

/// Static noun metadata from the lexicon database.
///
/// Provides lexical information for noun lookup including number
/// and semantic features. Nouns are keyed by their surface form,
/// with separate entries for singular and plural.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NounMetadata {
    /// The canonical form of the noun (usually singular).
    pub lemma: &'static str,

    /// The grammatical number of this surface form.
    /// "cat" → Singular, "cats" → Plural.
    pub number: Number,

    /// Semantic features including count/mass, animacy, and gender.
    pub features: &'static [Feature],
}

/// Static adjective metadata from the lexicon database.
///
/// Adjectives carry features that determine their semantic behavior
/// when combined with nouns (intersective, subsective, etc.) and
/// whether they support gradability and comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdjectiveMetadata {
    /// The base form of the adjective (positive degree).
    pub lemma: &'static str,

    /// Semantic features controlling modification behavior.
    /// See [`Feature::Intersective`], [`Feature::Subsective`], etc.
    pub features: &'static [Feature],
}

/// Canonical mapping for verb synonyms and antonyms.
///
/// Maps a verb to its canonical form for semantic normalization.
/// Antonyms are mapped with negative polarity, synonyms with positive.
/// Example: "despise" → ("hate", Positive), "love" → ("hate", Negative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalMapping {
    /// The canonical verb lemma this word maps to.
    pub lemma: &'static str,

    /// Whether the mapping preserves (Positive) or inverts (Negative) polarity.
    pub polarity: Polarity,
}

/// Morphological rule for derivational morphology.
///
/// Defines how suffixes transform words between categories.
/// Used for productive morphological patterns like "-ness" (adj → noun)
/// or "-ly" (adj → adv).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MorphologicalRule {
    /// The suffix that triggers this rule (e.g., "-ness", "-ly").
    pub suffix: &'static str,

    /// The part of speech or category produced (e.g., "noun", "adverb").
    pub produces: &'static str,
}
