# logicaffeine-lexicon

Foundational vocabulary types for the logicaffeine English-to-First-Order-Logic transpiler. This crate defines semantic and grammatical types used throughout the system for classifying English vocabulary.

## Features

| Feature | Description |
|---------|-------------|
| `default` | Compile-time only types (no runtime dependencies beyond logicaffeine-base) |
| `dynamic-lexicon` | Enables runtime JSON loading for development. Adds serde, serde_json, and rand dependencies. Provides the `runtime` module with `LexiconIndex` for runtime queries. |

## Core Types

All types are defined in the `types` module and re-exported at the crate root.

### Enumerations

| Type | Purpose |
|------|---------|
| `VerbClass` | Vendler's Aktionsart (State, Activity, Accomplishment, Achievement, Semelfactive) |
| `Sort` | Semantic type hierarchy (Entity, Physical, Animate, Human, Plant, Place, Time, Abstract, Information, Event, Celestial, Value, Group) |
| `Feature` | Lexical features (27 variants covering transitivity, control, semantics, adjective types, etc.) |
| `Time` | Temporal reference (Past, Present, Future, None) |
| `Aspect` | Grammatical aspect (Simple, Progressive, Perfect) |
| `Number` | Grammatical number (Singular, Plural) |
| `Gender` | Grammatical gender (Male, Female, Neuter, Unknown) |
| `Case` | Pronoun case (Subject, Object, Possessive) |
| `Definiteness` | Article class (Definite, Indefinite, Proximal, Distal) |
| `Polarity` | Lexical polarity (Positive, Negative) |

### Structures

| Type | Purpose |
|------|---------|
| `VerbEntry` | Verb entry with lemma, time, aspect, class (owned String) |
| `VerbMetadata` | Static verb metadata with `&'static str` references |
| `NounMetadata` | Static noun metadata with features |
| `AdjectiveMetadata` | Static adjective metadata with features |
| `CanonicalMapping` | Maps synonyms/antonyms to canonical forms with polarity |
| `MorphologicalRule` | Derivational morphology rules |

## Usage

### Basic Type Usage

```rust
use logicaffeine_lexicon::{VerbClass, Sort, Feature};

// Check verb aspectual properties
let class = VerbClass::Accomplishment;
assert!(class.is_durative());
assert!(class.is_telic());
assert!(!class.is_stative());

// Check semantic compatibility
let human = Sort::Human;
assert!(human.is_compatible_with(Sort::Animate));
assert!(human.is_compatible_with(Sort::Physical));
assert!(human.is_compatible_with(Sort::Entity));

// Parse features from strings
let feature = Feature::from_str("Transitive").unwrap();
```

### Runtime Lexicon (with `dynamic-lexicon` feature)

```rust
use logicaffeine_lexicon::runtime::{LexiconIndex, pluralize, past_tense};
use rand::thread_rng;

let lexicon = LexiconIndex::new();
let mut rng = thread_rng();

// Query verbs by class (pass string, not enum)
let states = lexicon.verbs_with_class("State");

// Query nouns by feature or sort
let animate_nouns = lexicon.nouns_with_feature("Animate");
let humans = lexicon.nouns_with_sort("Human");

// Get random entries for testing
let noun = lexicon.random_proper_noun(&mut rng);
let verb = lexicon.random_transitive_verb(&mut rng);

// Morphological helpers (standalone functions)
if let Some(n) = noun {
    let plural = pluralize(n);
}
if let Some(v) = verb {
    let past = past_tense(v);
}
```

## VerbClass (Vendler's Aktionsart)

Vendler's lexical aspect classes categorize verbs by their temporal properties:

| Class | Stative | Durative | Telic | Examples |
|-------|---------|----------|-------|----------|
| State | + | + | - | know, love, exist |
| Activity | - | + | - | run, swim, drive |
| Accomplishment | - | + | + | build, draw, write |
| Achievement | - | - | + | win, find, die |
| Semelfactive | - | - | - | knock, cough, blink |

Methods: `is_stative()`, `is_durative()`, `is_telic()`

## Sort System

The semantic type hierarchy for type checking. Compatibility flows upward through the hierarchy.

```
Entity (top)
├── Physical
│   └── Animate
│       ├── Human
│       └── Plant
├── Place
├── Time
├── Abstract
├── Information
├── Event
├── Celestial
├── Value
└── Group
```

Human is compatible with Animate, Physical, and Entity. Everything is compatible with Entity.

## Feature Categories

The 27 `Feature` variants grouped by category:

**Transitivity**
- `Transitive`, `Intransitive`, `Ditransitive`

**Control Theory**
- `SubjectControl`, `ObjectControl`, `Raising`

**Verb Semantics**
- `Opaque`, `Factive`, `Performative`, `Collective`, `Mixed`, `Distributive`, `Weather`, `Unaccusative`, `IntensionalPredicate`

**Noun Features**
- `Count`, `Mass`, `Proper`

**Gender**
- `Masculine`, `Feminine`, `Neuter`

**Animacy**
- `Animate`, `Inanimate`

**Adjective Types**
- `Intersective`, `NonIntersective`, `Subsective`, `Gradable`, `EventModifier`

## Runtime Module API

Available with the `dynamic-lexicon` feature.

### Types

- `LexiconIndex` - Main query interface
- `NounEntry` - JSON-deserialized noun with lemma, forms, features, sort
- `VerbEntry` - JSON-deserialized verb with lemma, class, forms, features
- `AdjectiveEntry` - JSON-deserialized adjective with lemma, regular flag, features

### Query Methods on `LexiconIndex`

```rust
// Noun queries
fn proper_nouns(&self) -> Vec<&NounEntry>
fn common_nouns(&self) -> Vec<&NounEntry>
fn nouns_with_feature(&self, feature: &str) -> Vec<&NounEntry>
fn nouns_with_sort(&self, sort: &str) -> Vec<&NounEntry>

// Verb queries
fn verbs_with_feature(&self, feature: &str) -> Vec<&VerbEntry>
fn verbs_with_class(&self, class: &str) -> Vec<&VerbEntry>
fn intransitive_verbs(&self) -> Vec<&VerbEntry>
fn transitive_verbs(&self) -> Vec<&VerbEntry>

// Adjective queries
fn adjectives_with_feature(&self, feature: &str) -> Vec<&AdjectiveEntry>
fn intersective_adjectives(&self) -> Vec<&AdjectiveEntry>
```

### Random Selection Methods

All require `&mut impl rand::Rng`:

```rust
fn random_proper_noun(&self, rng: &mut impl Rng) -> Option<&NounEntry>
fn random_common_noun(&self, rng: &mut impl Rng) -> Option<&NounEntry>
fn random_verb(&self, rng: &mut impl Rng) -> Option<&VerbEntry>
fn random_intransitive_verb(&self, rng: &mut impl Rng) -> Option<&VerbEntry>
fn random_transitive_verb(&self, rng: &mut impl Rng) -> Option<&VerbEntry>
fn random_adjective(&self, rng: &mut impl Rng) -> Option<&AdjectiveEntry>
fn random_intersective_adjective(&self, rng: &mut impl Rng) -> Option<&AdjectiveEntry>
```

### Morphology Functions

Standalone functions for inflection:

```rust
fn pluralize(noun: &NounEntry) -> String
fn present_3s(verb: &VerbEntry) -> String
fn past_tense(verb: &VerbEntry) -> String
fn gerund(verb: &VerbEntry) -> String
```

These functions first check for irregular forms in the entry's `forms` map, then apply regular morphological rules.

## Dependencies

- `logicaffeine-base` (required)
- `serde`, `serde_json`, `rand` (optional, with `dynamic-lexicon`)

## License

BUSL-1.1
