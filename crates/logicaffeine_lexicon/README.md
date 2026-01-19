# logicaffeine-lexicon

Linguistic type definitions and vocabulary management for the Logicaffeine English-to-First-Order-Logic transpiler.

Part of the [Logicaffeine](https://logicaffeine.com) project.

## Overview

This crate provides:

- **Type definitions** for linguistic categories (verb classes, semantic sorts, grammatical features)
- **Metadata structures** for zero-copy lexicon lookups
- **Optional runtime lexicon** for development iteration (via `dynamic-lexicon` feature)

## Quick Start

```rust
use logicaffeine_lexicon::{VerbClass, Feature, Sort};

// Check verb aspectual properties (Vendler classification)
let class = VerbClass::Activity;
assert!(!class.is_stative());  // Activities are dynamic
assert!(class.is_durative());   // Activities have duration
assert!(!class.is_telic());     // Activities have no inherent endpoint

// Parse features from strings
let feature = Feature::from_str("Transitive");
assert_eq!(feature, Some(Feature::Transitive));

// Check semantic sort compatibility (subsumption hierarchy)
assert!(Sort::Human.is_compatible_with(Sort::Animate));  // Human ⊆ Animate
assert!(Sort::Human.is_compatible_with(Sort::Entity));   // Everything ⊆ Entity
assert!(!Sort::Animate.is_compatible_with(Sort::Human)); // Not vice versa
```

## Core Types

### Grammatical Enumerations

| Type | Variants | Description |
|------|----------|-------------|
| `Time` | `Past`, `Present`, `Future`, `None` | Temporal reference for verb tense |
| `Aspect` | `Simple`, `Progressive`, `Perfect` | Viewpoint aspect (grammatical) |
| `Number` | `Singular`, `Plural` | Grammatical number |
| `Gender` | `Male`, `Female`, `Neuter`, `Unknown` | Grammatical gender |
| `Case` | `Subject`, `Object`, `Possessive` | Grammatical case (pronouns) |
| `Definiteness` | `Definite`, `Indefinite`, `Proximal`, `Distal` | Article/demonstrative definiteness |
| `Polarity` | `Positive`, `Negative` | For synonym/antonym canonical mappings |

### VerbClass (Vendler Classification)

Vendler's lexical aspect classes (Aktionsart) determine how verbs interact with temporal adverbials and aspect markers.

| Class | Static | Durative | Telic | Examples |
|-------|--------|----------|-------|----------|
| `State` | + | + | - | know, love, exist |
| `Activity` | - | + | - | run, swim, drive |
| `Accomplishment` | - | + | + | build, draw, write |
| `Achievement` | - | - | + | win, find, die |
| `Semelfactive` | - | - | - | knock, cough, blink |

```rust
use logicaffeine_lexicon::VerbClass;

let class = VerbClass::Accomplishment;
assert!(!class.is_stative());  // Dynamic (involves change)
assert!(class.is_durative());   // Takes time to complete
assert!(class.is_telic());      // Has an inherent endpoint
```

### Sort (Semantic Type Hierarchy)

Sorts provide semantic type checking. A `Human` can fill an `Animate` slot, but not vice versa.

```
                Entity
               /      \
          Physical    Abstract
         /       \         \
     Animate    Place    Information
    /      \
Human    Plant           Event

(Also: Time, Celestial, Value, Group)
```

Compatibility follows subsumption:
- `Human ⊆ Animate ⊆ Physical ⊆ Entity`
- `Plant ⊆ Animate ⊆ Physical ⊆ Entity`
- Everything ⊆ `Entity`

```rust
use logicaffeine_lexicon::Sort;

// Human can be used where Animate is expected
assert!(Sort::Human.is_compatible_with(Sort::Animate));

// But Animate cannot be used where Human is expected
assert!(!Sort::Animate.is_compatible_with(Sort::Human));
```

### Feature (27 Variants)

Lexical features encode grammatical and semantic properties of words.

**Transitivity**
- `Transitive` — requires direct object: "see", "hit"
- `Intransitive` — no object: "sleep", "arrive"
- `Ditransitive` — two objects: "give", "tell"

**Control Theory**
- `SubjectControl` — subject controls embedded PRO: "promise", "try"
- `ObjectControl` — object controls embedded PRO: "persuade", "force"
- `Raising` — no theta-role to surface subject: "seem", "appear"

**Semantic**
- `Opaque` — intensional context: "believe", "want"
- `Factive` — presupposes complement truth: "know", "regret"
- `Performative` — uttering performs the action: "promise", "declare"
- `Collective` — requires group subject: "gather", "meet"
- `Mixed` — collective or distributive: "lift", "carry"
- `Distributive` — applies to each individual: "sleep", "smile"
- `Weather` — impersonal, expletive subject: "rain", "snow"
- `Unaccusative` — subject is underlying theme: "arrive", "melt"
- `IntensionalPredicate` — operates on intensions: "believe", "hope"

**Noun**
- `Count` — individuated, takes numerals: "cat", "idea"
- `Mass` — requires measure phrases: "water", "rice"
- `Proper` — rigid designator: "Socrates", "Paris"

**Gender**
- `Masculine`, `Feminine`, `Neuter`

**Animacy**
- `Animate` — capable of self-initiated action
- `Inanimate` — not sentient

**Adjective**
- `Intersective` — set intersection: "red ball" = red ∧ ball
- `NonIntersective` — not intersection: "fake gun" ≠ fake ∧ gun
- `Subsective` — relative to comparison class: "skillful surgeon"
- `Gradable` — supports degree: "tall", "expensive"
- `EventModifier` — modifies events: "careful", "deliberate"

### Metadata Structs

These structs provide zero-copy access to lexicon data:

```rust
use logicaffeine_lexicon::{VerbMetadata, VerbClass, Feature, Time, Aspect};

// Returned from compile-time generated lookup functions
let verb: VerbMetadata = VerbMetadata {
    lemma: "give",
    class: VerbClass::Achievement,
    time: Time::None,
    aspect: Aspect::Simple,
    features: &[Feature::Ditransitive],
};
```

| Struct | Fields |
|--------|--------|
| `VerbMetadata` | `lemma`, `class`, `time`, `aspect`, `features` |
| `NounMetadata` | `lemma`, `number`, `features` |
| `AdjectiveMetadata` | `lemma`, `features` |
| `VerbEntry` | Owned version for irregular verb lookups |
| `CanonicalMapping` | For synonym/antonym normalization |
| `MorphologicalRule` | Derivational morphology patterns |

## Features

| Feature | Description |
|---------|-------------|
| (default) | Type definitions only, no dependencies beyond `logicaffeine-base` |
| `dynamic-lexicon` | Runtime JSON loading, queries, morphology functions |

```toml
# Types only (for compile-time codegen)
[dependencies]
logicaffeine-lexicon = "0.6"

# With runtime lexicon (for development)
[dependencies]
logicaffeine-lexicon = { version = "0.6", features = ["dynamic-lexicon"] }
```

## Runtime Lexicon (Optional)

With the `dynamic-lexicon` feature, you get:

### LexiconIndex

```rust
use logicaffeine_lexicon::runtime::LexiconIndex;

let lexicon = LexiconIndex::new();

// Query by feature
let transitive = lexicon.verbs_with_feature("Transitive");
let animate = lexicon.nouns_with_feature("Animate");

// Query by category
let proper_nouns = lexicon.proper_nouns();
let common_nouns = lexicon.common_nouns();
let state_verbs = lexicon.verbs_with_class("State");

// Random selection (useful for testing/generation)
let mut rng = rand::thread_rng();
let random_verb = lexicon.random_transitive_verb(&mut rng);
```

### Morphological Functions

```rust
use logicaffeine_lexicon::runtime::{NounEntry, VerbEntry, pluralize, present_3s, past_tense, gerund};
use std::collections::HashMap;

// Regular pluralization
let dog = NounEntry {
    lemma: "dog".to_string(),
    forms: HashMap::new(),
    features: vec![],
    sort: None,
};
assert_eq!(pluralize(&dog), "dogs");

// Irregular forms (via forms map)
let mouse = NounEntry {
    lemma: "mouse".to_string(),
    forms: [("plural".to_string(), "mice".to_string())].into(),
    features: vec![],
    sort: None,
};
assert_eq!(pluralize(&mouse), "mice");

// Verb conjugation
let walk = VerbEntry {
    lemma: "walk".to_string(),
    class: "Activity".to_string(),
    forms: HashMap::new(),
    features: vec![],
};
assert_eq!(present_3s(&walk), "walks");
assert_eq!(past_tense(&walk), "walked");
assert_eq!(gerund(&walk), "walking");
```

**Morphological rules applied:**

| Function | Rule | Example |
|----------|------|---------|
| `pluralize` | sibilant + `-es` | box → boxes |
| `pluralize` | consonant + y → `-ies` | city → cities |
| `present_3s` | sibilant/o + `-es` | go → goes |
| `past_tense` | e + `-d` | love → loved |
| `past_tense` | consonant + y → `-ied` | carry → carried |
| `gerund` | drop e + `-ing` | make → making |

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    logicaffeine_language                         │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │  build.rs: generates lexicon.rs from lexicon.json           ││
│  │           (compile-time code generation)                    ││
│  └─────────────────────────────────────────────────────────────┘│
│                              │                                   │
│                              ▼                                   │
│                    lexicon.rs (generated)                        │
│                    - lookup_verb("run") → VerbMetadata          │
│                    - lookup_noun("cat") → NounMetadata          │
└─────────────────────────────────────────────────────────────────┘
                               │
                               │ uses types from
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                    logicaffeine_lexicon                          │
│  ┌────────────────────────┐  ┌────────────────────────────────┐ │
│  │  types.rs              │  │  runtime.rs (optional)         │ │
│  │  - VerbClass           │  │  - LexiconIndex                │ │
│  │  - Sort                │  │  - pluralize(), past_tense()   │ │
│  │  - Feature             │  │  - JSON deserialization        │ │
│  │  - *Metadata structs   │  │  (dynamic-lexicon feature)     │ │
│  └────────────────────────┘  └────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

**Compile-time mode** (default): `logicaffeine_language`'s build script generates type-safe Rust lookup functions from `lexicon.json`. This crate provides only the type definitions those functions return.

**Runtime mode** (`dynamic-lexicon`): Embeds `lexicon.json` and parses it at runtime. Useful during development when frequently editing the lexicon, as it avoids full recompilation.

## License

Business Source License 1.1 (BUSL-1.1)

- **Free** for individuals and organizations with <25 employees
- **Commercial license** required for organizations with 25+ employees offering Logic Services
- **Converts to MIT** on December 24, 2029

See [LICENSE](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md) for full terms.
