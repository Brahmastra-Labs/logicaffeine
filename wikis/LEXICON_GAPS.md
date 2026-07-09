# Lexicon gap analysis — what WordNet/nltk model that LOGOS does not

Generated from probing WordNet against the LOGOS lexicon schema. `--enrich` surfaces all of
this live per word (and flags the ⚠ gaps). The question this answers: **which of WordNet's
distinctions should LOGOS adopt?**

Three tiers: **(A) ground now** — no schema change, just feed an existing field; **(B) new
category** — a real FOL-relevant distinction LOGOS can't express (needs a Rust schema + parser
change → a design decision); **(C) out of scope** for an English→FOL transpiler.

## Tier A — ground existing fields from WordNet (no schema change, do via `--enrich`)

| WordNet signal | Feeds LOGOS | Today | Win |
|---|---|---|---|
| Verb **frames** (`Somebody ----s somebody something`) | `Transitive`/`Intransitive`/`Ditransitive` features | set by hand | **free accuracy** — give→Ditransitive, kill→Transitive, sleep→Intransitive, all derivable |
| **Supersense** (lexname: `noun.animal`, `noun.group`…) | noun `sort` | guessed | candidate sorts per sense (confirm by sentence) |
| **Hypernyms** (dog→canine→mammal→animal) | `axioms.nouns.hypernyms` | ~11 hardcoded | populate at scale |
| Part **meronyms** (car→{engine, door, window…}) | `ontology.part_whole` | ~9 hardcoded | populate at scale |
| Verb **entailment** (buy⊨pay) / **cause** (kill→die) | `axioms.verbs` | ~4 hardcoded | populate at scale |
| **Antonyms** (tall↔short) | verb antonyms / canonical | partial | populate |
| **Derivation** (hunter←hunt) | noun `derivation` | manual | suggest (noisy — confirm) |

Caveats baked into `--enrich`: **sort/sense is ambiguous** (genus → Abstract *and* Group; otter
→ fur *and* animal) so it offers candidates, never an answer; and **WordNet has no coverage of
scientific/Latin terms** (`nipponicum`, `photosynthesize` → nothing).

## Tier B — genuine NEW categories LOGOS cannot express (need a schema/parser change — DECIDE)

1. **Relational / pertainymic adjectives.** WordNet *pertainyms* identify them precisely:
   `coastal`→`coast`, `dental`→`tooth`, `nuclear`→`nucleus`. These do **not predicate**
   ("*the region is coastal" is odd) and in FOL relate a noun to another noun, unlike
   intersective ("red") or subsective ("tall") adjectives. **LOGOS has no `Relational` feature**
   — it would tag `coastal` as `Subsective`, which is wrong. *Proposed:* add `Feature::Relational`
   (types.rs) + build.rs wiring + parser handling for how a relational adjective composes into
   FOL. Clean, well-bounded, and WordNet gives us the data + detection for free.

2. **Adjective ↔ scale/Dimension link.** WordNet's *attribute* relation ties a gradable
   adjective to its scale: `tall`→`stature`, `heavy`→`weight`, `hot`→`temperature`. LOGOS has a
   `Gradable` feature and a `Dimension` enum (Length/Weight/Temperature…) for units, but **no
   link from an adjective to its dimension** — so it can't connect "tall" to a height measure.
   *Proposed:* an optional `scale`/`dimension` on gradable adjectives, tying comparatives and
   measure phrases together. Medium value, medium effort.

3. (Smaller) **Troponymy** (manner: march⊂walk) and adjective **satellite/similar-to** clusters —
   could refine manner adverbials and adjective scales. Low priority.

## Tier C — out of scope for English→FOL

- **Sentiment** (SentiWordNet polarity), **pronunciation** (CMUdict), full **FrameNet/PropBank**
  predicate-argument structure — rich, but not what a logic transpiler consumes.

## Recommendation

- **Adopt Tier A now**: `--enrich` already surfaces it; wire the loop to *propose* hypernyms /
  meronyms / entailment / transitivity from WordNet during a word-add (confirm, then apply).
- **Tier B #1 (relational adjectives)** is the strongest real schema gap — small, FOL-relevant,
  and fully supported by WordNet detection. Worth a proper RED-test-driven addition.
- **Tier B #2 (adj↔dimension)** is worth doing if degree/measure semantics matter near-term.
- These are Rust schema + parser changes (touch `types.rs`, `build.rs`, the parser, the suite) —
  i.e. design decisions, human-supervised, not auto-applied.
