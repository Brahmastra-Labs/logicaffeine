# logicaffeine-lexicon

The vocabulary the LOGOS parser reads English with: verbs tagged with their Vendler class and transitivity, nouns with number, gender and animacy, adjectives with their semantic kind, and the types those lookups return. Rather than load a dictionary at startup, the vocabulary is one JSON document that `logicaffeine-language` compiles into lookup tables at build time, so looking a word up parses nothing at run time. This crate holds the data and the types, not the generated lookups; its optional runtime index serves the web app's exercise generator, not the parser.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 0. The default build adds no third-party dependencies; `dynamic-lexicon` brings in `serde`, `serde_json` and `rand`.

## Role in the workspace

This crate is the vocabulary substrate for the Logic-mode pipeline (see [logic-mode.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/logic-mode.md)). `logicaffeine-language` takes it as a build-dependency and generates Rust lookup tables from `LEXICON_JSON` (`assets/lexicon.json`). The generated `lookup_verb_db`, `lookup_noun_db`, `lookup_adjective_db` and their siblings return the `Copy` metadata structs declared here, whose lemmas and feature lists are `&'static` data compiled into the binary. Changing a word means rebuilding `logicaffeine-language`.

The `dynamic-lexicon` runtime module parses the same JSON in-process into a `LexiconIndex` you can query by feature, sort and class. The web app's exercise generator uses it to pick words for its templates, and on `wasm32` it fetches the document at run time with `LexiconIndex::from_json` instead of embedding it.

## Public API

All `types` items are re-exported at the crate root, alongside `LEXICON_JSON`, the vocabulary document itself.

Grammatical enums (all `Copy`): `Time` (Past/Present/Future/None), `Aspect` (Simple/Progressive/Perfect), `Number` (Singular/Plural), `Gender` (Male/Female/Neuter/Unknown), `Case` (Subject/Object/Possessive), `Definiteness` (Definite/Indefinite/Proximal/Distal), `Polarity` (Positive/Negative).

`VerbClass`: Vendler's Aktionsart (`State`, `Activity` (`#[default]`), `Accomplishment`, `Achievement`, `Semelfactive`):

```text
pub fn is_stative(&self) -> bool;   // State only
pub fn is_durative(&self) -> bool;  // State | Activity | Accomplishment
pub fn is_telic(&self) -> bool;     // Accomplishment | Achievement
```

`Sort`: a semantic hierarchy (`Entity`, `Physical`, `Animate`, `Human`, `Plant`, `Place`, `Time`, `Abstract`, `Information`, `Event`, `Celestial`, `Value`, `Signal`, `Group`):

```text
pub fn is_compatible_with(self, other: Sort) -> bool;  // subsumption: Human/Plant ⊆ Animate ⊆ Physical ⊆ Entity; * ⊆ Entity
pub fn is_occasion(self) -> bool;                      // soft-typed occurrences; true only for Event
```

`Feature`: the lexical features, by group.

- verb transitivity: Transitive, Intransitive, Ditransitive
- control: SubjectControl, ObjectControl, Raising
- semantics: Opaque, Factive, Performative, Collective, Mixed, Distributive, Weather, Unaccusative, IntensionalPredicate, Resultative, Perception, Relevance
- nouns: Count, Mass, Proper
- gender: Masculine, Feminine, Neuter
- animacy: Animate, Inanimate
- adjectives: Intersective, NonIntersective, Subsective, Gradable, EventModifier, Relational, Vague, Decreasing

```text
pub fn from_str(s: &str) -> Option<Feature>;  // case-sensitive, by variant name
```

Lookup payloads are `Copy` structs holding `&'static str` lemmas and `&'static [Feature]`, so a lookup copies no strings:

```text
pub struct VerbMetadata { pub lemma: &'static str, pub class: VerbClass, pub time: Time, pub aspect: Aspect, pub features: &'static [Feature] }
pub struct NounMetadata { pub lemma: &'static str, pub number: Number, pub features: &'static [Feature] }
pub struct AdjectiveMetadata { pub lemma: &'static str, pub features: &'static [Feature] }
```

`VerbEntry` (in `types`) is the *owned* result of an inflected-form lookup (`lemma: String`, `time`, `aspect`, `class`). `CanonicalMapping` (synonym or antonym → `lemma` + `polarity`) and `MorphologicalRule` (`suffix` → `produces` category) round out the set.

```rust
use logicaffeine_lexicon::{VerbClass, Feature, Sort};

let class = VerbClass::Accomplishment;
assert!(!class.is_stative() && class.is_durative() && class.is_telic());
assert_eq!(Feature::from_str("Transitive"), Some(Feature::Transitive));
assert!(Sort::Human.is_compatible_with(Sort::Animate));
assert!(!Sort::Animate.is_compatible_with(Sort::Human));
```

### Runtime module (feature `dynamic-lexicon`)

`LexiconIndex::new()` / `Default` parses the embedded JSON once, and `LexiconIndex::from_json(&str) -> Result<LexiconIndex, serde_json::Error>` parses a caller-supplied document of the same format. Both build the index from the module's own serde types (`LexiconData`, `NounEntry`, `VerbEntry`, `AdjectiveEntry`), which are deliberately distinct from the compile-time `types`: `runtime::VerbEntry` (an owned serde struct) and `types::VerbEntry` (an owned lookup result) are different types that share a name. Feature, sort and class matching is case-insensitive.

Query methods returning `Vec<&Entry>`: `proper_nouns`, `common_nouns`, `nouns_with_feature`, `nouns_with_sort`, `verbs_with_feature`, `verbs_with_class`, `intransitive_verbs`, `transitive_verbs` (Transitive ∪ Ditransitive), `adjectives_with_feature`, `intersective_adjectives`. The pickers `random_proper_noun`, `random_common_noun`, `random_verb`, `random_intransitive_verb`, `random_transitive_verb`, `random_adjective` and `random_intersective_adjective` take `&mut impl rand::Rng` and return `Option<&Entry>`.

Free morphology functions check the entry's irregular `forms` map first, then fall back to regular rules: `pluralize` (box→boxes, city→cities), `present_3s` (go→goes), `past_tense` (love→loved, carry→carried), and `gerund` (make→making). `gerund` does not double consonants, so supply an irregular form for verbs like "run".

```rust
use logicaffeine_lexicon::runtime::LexiconIndex;
use logicaffeine_lexicon::LEXICON_JSON;

let lex = LexiconIndex::new();
assert!(lex.proper_nouns().iter().any(|n| n.lemma == "John"));
assert!(lex.intersective_adjectives().iter().any(|a| a.lemma == "Happy"));

let fetched = LexiconIndex::from_json(LEXICON_JSON).unwrap();
assert_eq!(fetched.common_nouns().len(), lex.common_nouns().len());
assert!(LexiconIndex::from_json("{").is_err());
```

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `dynamic-lexicon` | off | The `runtime` module: `LexiconIndex`, its serde entry types and the morphology functions, parsing `LEXICON_JSON` or a caller-supplied document at run time. Pulls in `serde`, `serde_json` and `rand`. docs.rs builds with it on. |

Without it, the crate is the type definitions plus `LEXICON_JSON`.

## Dependencies

Internal: none.

External (all `optional`, enabled only by `dynamic-lexicon`): `serde` (derive), `serde_json`, `rand` 0.8.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_lexicon/CHANGELOG.md)
