# logicaffeine-lexicon

English vocabulary types plus compile-time and runtime lexicon lookup for the Logicaffeine English-to-First-Order-Logic transpiler. It defines the linguistic type system that lexicon lookups return, and optionally embeds and parses the lexicon JSON at runtime.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 1 — depends on logicaffeine_base. Consumed by logicaffeine_language.

## Role in the workspace

This crate is the vocabulary substrate for the Logic-mode pipeline (see [../../new_docs/logic-mode.md](../../new_docs/logic-mode.md)). In the default (compile-time) mode, `logicaffeine-language` generates Rust code from `crates/logicaffeine_language/assets/lexicon.json` at build time; the generated `lookup_verb`/`lookup_noun`/… functions hand back the `&'static` metadata structs declared here, so lookups carry zero parsing cost. The optional runtime mode parses that same JSON in-process for faster lexicon edit-compile cycles.

## Public API

All `types` items are re-exported at the crate root.

Grammatical enums (all `Copy`): `Time` (Past/Present/Future/None), `Aspect` (Simple/Progressive/Perfect), `Number` (Singular/Plural), `Gender` (Male/Female/Neuter/Unknown), `Case` (Subject/Object/Possessive), `Definiteness` (Definite/Indefinite/Proximal/Distal), `Polarity` (Positive/Negative).

`VerbClass` — Vendler's Aktionsart, 5 variants (`State`, `Activity` (`#[default]`), `Accomplishment`, `Achievement`, `Semelfactive`):

```rust
pub fn is_stative(&self) -> bool;   // State only
pub fn is_durative(&self) -> bool;  // State | Activity | Accomplishment
pub fn is_telic(&self) -> bool;     // Accomplishment | Achievement
```

`Sort` — a 14-member semantic hierarchy (`Entity`, `Physical`, `Animate`, `Human`, `Plant`, `Place`, `Time`, `Abstract`, `Information`, `Event`, `Celestial`, `Value`, `Signal`, `Group`):

```rust
pub fn is_compatible_with(self, other: Sort) -> bool;  // subsumption: Human/Plant ⊆ Animate ⊆ Physical ⊆ Entity; * ⊆ Entity
pub fn is_occasion(self) -> bool;                      // soft-typed occurrences; true only for Event
```

`Feature` — 34 lexical features spanning verb transitivity (Transitive, Intransitive, Ditransitive), control theory (SubjectControl, ObjectControl, Raising), semantics (Opaque, Factive, Performative, Collective, Mixed, Distributive, Weather, Unaccusative, IntensionalPredicate, Resultative, Perception, Relevance), nouns (Count, Mass, Proper), gender (Masculine, Feminine, Neuter), animacy (Animate, Inanimate), and adjectives (Intersective, NonIntersective, Subsective, Gradable, EventModifier, Relational, Vague, Decreasing):

```rust
pub fn from_str(s: &str) -> Option<Feature>;  // case-sensitive, by variant name
```

Lookup payloads — zero-copy `Copy` structs holding `&'static str` lemmas and `&'static [Feature]`:

```rust
pub struct VerbMetadata { pub lemma: &'static str, pub class: VerbClass, pub time: Time, pub aspect: Aspect, pub features: &'static [Feature] }
pub struct NounMetadata { pub lemma: &'static str, pub number: Number, pub features: &'static [Feature] }
pub struct AdjectiveMetadata { pub lemma: &'static str, pub features: &'static [Feature] }
```

`VerbEntry` (in `types`) is the *owned* result of an inflected-form lookup (`lemma: String`, `time`, `aspect`, `class`). `CanonicalMapping` (synonym/antonym → `lemma` + `polarity`) and `MorphologicalRule` (`suffix` → `produces` category) round out the set.

```rust
use logicaffeine_lexicon::{VerbClass, Feature, Sort};

let class = VerbClass::Accomplishment;
assert!(!class.is_stative() && class.is_durative() && class.is_telic());
assert_eq!(Feature::from_str("Transitive"), Some(Feature::Transitive));
assert!(Sort::Human.is_compatible_with(Sort::Animate));
assert!(!Sort::Animate.is_compatible_with(Sort::Human));
```

### Runtime module (feature `dynamic-lexicon`)

`LexiconIndex::new()` / `Default` parses the embedded JSON once into its own serde types — `LexiconData`, `NounEntry`, `VerbEntry`, `AdjectiveEntry`. These are deliberately distinct from the compile-time `types`; in particular `runtime::VerbEntry` (owned serde struct) and `types::VerbEntry` (owned lookup result) are different types sharing a name. Feature/sort/class matching is case-insensitive.

Query methods returning `Vec<&Entry>`: `proper_nouns`, `common_nouns`, `nouns_with_feature`, `nouns_with_sort`, `verbs_with_feature`, `verbs_with_class`, `intransitive_verbs`, `transitive_verbs` (Transitive ∪ Ditransitive), `adjectives_with_feature`, `intersective_adjectives`. Each has a matching `random_*` picker taking `&mut impl rand::Rng` and returning `Option<&Entry>`.

Free morphology functions check the entry's irregular `forms` map first, then fall back to regular rules: `pluralize` (box→boxes, city→cities), `present_3s` (go→goes), `past_tense` (love→loved, carry→carried), `gerund` (make→making; no consonant doubling — supply an irregular form for verbs like "run").

```rust
use logicaffeine_lexicon::runtime::LexiconIndex;

let lex = LexiconIndex::new();
let transitive = lex.transitive_verbs();
let proper = lex.proper_nouns();
let states = lex.verbs_with_class("State");
```

## Feature flags

| Feature | Default | Gates |
|---------|---------|-------|
| (none) | ✓ | Type definitions only. No runtime parsing, no extra dependencies. |
| `dynamic-lexicon` | | Enables the `runtime` module: pulls in `serde`, `serde_json`, `rand` and embeds `lexicon.json` via `include_str!`, parsing it at runtime. docs.rs builds with this on. |

## Dependencies

Internal: `logicaffeine-base`.

External (all `optional`, enabled only by `dynamic-lexicon`): `serde` (derive), `serde_json`, `rand` 0.8.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
