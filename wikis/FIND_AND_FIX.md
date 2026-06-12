# Prompt: find and fix one compiler bug from real Wikipedia prose

Paste the block below to an agent. It surfaces a real bug via the wiki scripts, fixes it
under the right amount of rigor, and **escalates only when something is genuinely hard.**
Do not get a new article until all existing wikis compile.

The single most important calibration, learned the hard way:

> **Adding vocabulary to the lexicon is cake.** It is the *easiest* class of fix — you are
> slotting a value into a known struct. **Do not escalate a missing word.** The hard things
> are real semantics (multi-world / Kripke, new primitives, scope), not vocabulary. If you
> find yourself writing a 3-option escalation memo for a missing noun, you have misread the
> system — go back and learn the lexicon's shape (below).

---

You are improving the LOGOS English→FOL compiler by finding and fixing **one** real bug
surfaced by real Wikipedia prose. Work in `/Users/tristenharr/logicaffeine`.
**Do not run git.** Read `wikis/README.md` first.

Your prime directive has two halves:
- **Fix what is fixable**, end to end. Vocabulary gaps are the common case and are routine.
- **Escalate only genuine judgment calls** — contested *meaning*, real design forks, or a
  word the data model genuinely cannot express. A good escalation is a success; a *needless*
  one (bailing on a word-add) is the failure mode this doc exists to prevent.

## 0. Know the lexicon BEFORE you touch it (read types, not the 2,356-line JSON)

The leverage is not in reading every entry — it is in ~10 Rust structs + the enums (listed in
full below) + **63** generated lookup functions that define what the data can *mean*. Learn
the shape once and any word-add becomes "pick the section, pick the enum values, mirror a
sibling." This section is the full map; everything in it was verified against the crate.

**The pipeline:** `lexicon.json` → `build.rs` deserializes each section into a struct →
code-gens **63** functions into `$OUT_DIR/lexicon_data.rs` (plus `mwe_data.rs`,
`ontology_data.rs`, `axiom_data.rs`) → lexer/parser call those functions. Edit JSON, run
`cargo build`, the tables regenerate (if a table looks stale, `touch` the JSON to force it).

**Source of truth — read these first (they cannot drift; this doc can):**
- Data: `crates/logicaffeine_language/assets/lexicon.json` (24 top-level sections).
- Deserialization structs + codegen: `crates/logicaffeine_language/build.rs` (`struct
  NounDefinition` ~84, `NounForms` ~111, `VerbDefinition` ~56, `VerbForms` ~72,
  `AdjectiveDefinition` ~117, `MorphologicalRule` ~104, `Morphology` ~126, `MweEntry` ~133,
  `OntologyData`/`PartWholeEntry`, `AxiomData`; `fn derive_irregular_plurals` ~650,
  `generate_singularize` ~922, `generate_is_check` ~956). Grep here to see which function each
  datum drives.
- **Enums live in the separate `logicaffeine_lexicon` crate**:
  `crates/logicaffeine_lexicon/src/types.rs` — `Definiteness` (7), `Time` (20), `Aspect` (33),
  `VerbClass` (44), `Sort` (88), `Number` (146), `Gender` (154), `Case` (167), `Polarity`
  (179), `Feature` (193). Two more live in `logicaffeine_language`: `Dimension`
  (`src/ast/logic.rs:49`), `PresupKind` (`src/token.rs:44`). (`logicaffeine_language` re-exports
  the first set as `crate::lexicon::…`.)
- Consumers: `src/lexer.rs` `fn classify_word` (~2037; POS assignment — unknown content words
  hit the `Adjective` fallback at ~2496); `src/parser/mod.rs` `is_plural_noun` (~1006) /
  `singularize_noun` (~1018).
- Compile API for quick checks / tests: `logicaffeine_language::{compile, compile_simple,
  compile_kripke}` (`src/compile.rs:45`).

### Given a word, WHERE does it belong? (routing guide)

Decide the word's role *in the sentence*, then go to the section. (The harness's
`proposal.lexicon_entry` POS is a **suffix guess — verify it**: it mislabels "now"→Noun (it's
an adverb) and "up"→Noun (a particle).)

| The word is… | Section | What to fill (enum in **bold**) |
|---|---|---|
| a common noun (dog, genus, idea) | `nouns` | `lemma` (Capitalized); `forms.plural` only if irregular/Latin-`-s`; `sort` (**Sort**); `features` (**Feature**: gender/animacy/Count/Mass) |
| a proper name (one word) | `nouns` | `lemma`, `features:["Proper"]` (+ **Gender** feature if relevant) |
| a verb | `verbs` | `lemma`, `class` (**VerbClass**); `forms{present3s,past,participle,gerund}` if irregular else `regular:true`; `features` (**Feature**: transitivity/control/semantic) |
| an adjective | `adjectives` | `lemma`, `regular` (bool), `features` (**Feature**: Intersective/NonIntersective/Subsective/Gradable/EventModifier) |
| a preposition | `prepositions` | the word (string) |
| a plain adverb (quickly) | `adverbs` | the word (string) |
| a scope-bearing adverb (almost, allegedly) | `scopal_adverbs` | the word (string) |
| a temporal adverb (yesterday, now) | `temporal_adverbs` | the word (string) |
| an `-ly` word that is NOT an adverb (friendly) | `not_adverbs` | the word (string) |
| a phrasal-verb particle (up, off) | `particles` | the word (string) |
| a phrasal verb with idiomatic meaning (give up) | `phrasal_verbs` | `"verb_particle": {lemma, class (`**VerbClass**`)}` |
| a fixed multi-word expression (fire engine) | `multi_word_expressions` | `{pattern:[words], lemma, pos (Noun/Verb/Preposition/Conjunction/Quantifier), class?, features?}` |
| a measurement unit (inch) | `units` | `"word": "`**Dimension**`"` |
| a spelled-out number (seven) | `number_words` | `"word": <int>` |
| a pronoun | `pronouns` | `{word, gender (`**Gender**`), number (`**Number**`), case (`**Case**`)}` |
| an article/determiner | `articles` | `"word": "`**Definiteness**`"` |
| a tense auxiliary (will) | `auxiliaries` | `"word": "`**Time**`"` |
| a quantifier / logical keyword (every, and, if) | `keywords` | `"word": "TokenType"` |
| a verb that triggers presupposition (stop, regret) | `presupposition_triggers` | `"word": "`**PresupKind**`"` |
| a noun/adj homonym mis-read as a past-tense verb (bit, red) | `disambiguation_not_verbs` | the word (string) |
| a productive derivational suffix (`-er`→Agent) | `morphological_rules` | `{suffix, base_pos, relation}` |
| a meaning postulate (bachelor⊨unmarried; dog⊑animal; fake=privative) | `axioms` | `nouns{entails[],hypernyms[]}` / `adjectives{type}` / `verbs{entails,manner[]}` |
| a part-whole or predicate-sort fact | `ontology` | `part_whole[{whole,parts[]}]` / `predicate_sorts{pred: `**Sort**`}` |
| an inflection irregularity (silent-e, stem exception) | `morphology` | add to `needs_e_ing`/`needs_e_ed`/`stemming_exceptions` |

### Per-section reference (shape · generated fn · purpose)

| Section | Shape (struct) | Generated fn(s) | Purpose |
|---|---|---|---|
| `keywords` | `{word: TokenName}` | `lookup_keyword` | quantifiers/connectives/modals → token |
| `pronouns` | `{word,gender,number,case}` | `lookup_pronoun` | pronoun features for anaphora/agreement |
| `articles` | `{word: Definiteness}` | `lookup_article` | the/a/this/that → definiteness |
| `auxiliaries` | `{word: Time}` | `lookup_auxiliary` | will/did → tense |
| `presupposition_triggers` | `{word: PresupKind}` | `lookup_presup_trigger` | stop/regret → presupposition |
| `number_words` | `{word: u32}` | `word_to_number` | one…ten → cardinal |
| `verbs` | `VerbDefinition{lemma,class,forms,regular,features,synonyms,antonyms}` | `lookup_verb_db`, `lookup_verb_class`, `lookup_irregular_verb`, `is_*_verb` (ditransitive/subject_control/object_control/raising/opaque/collective/mixed/distributive/intensional/base) | verbs: arity, Vendler class, control |
| `nouns` | `NounDefinition{lemma,forms{plural},features,sort,derivation}` | `is_common_noun`, `singularize`, `is_irregular_plural`, `lookup_noun_db`, `lookup_sort`, `is_{male,female,neuter}_noun`, `is_{male,female}_name`, `lookup_noun_derivation`, `is_agentive_noun` | common nouns, number, sort, gender |
| `adjectives` | `AdjectiveDefinition{lemma,regular,features}` | `is_adjective`, `is_non_intersective`, `is_subsective`, `is_gradable_adjective`, `is_event_modifier_adjective`, `lookup_adjective_db` | adjective composition mode |
| `morphological_rules` | `MorphologicalRule{suffix,base_pos,relation}` | `get_morphological_rules` | productive derivation (-er→Agent) |
| `prepositions` | `[string]` | `is_preposition` | closed-class membership |
| `adverbs` | `[string]` | `is_adverb` | manner/degree adverbs |
| `scopal_adverbs` | `[string]` | `is_scopal_adverb` | scope-island adverbs |
| `temporal_adverbs` | `[string]` | `is_temporal_adverb` | time-locating adverbs |
| `particles` | `[string]` | `is_particle` | phrasal-verb particles |
| `phrasal_verbs` | `{v_p:{lemma,class}}` | `lookup_phrasal_verb` | give up→Surrender |
| `not_adverbs` | `[string]` | `is_not_adverb` | `-ly` words that aren't adverbs |
| `noun_patterns` | `[string]` | `is_noun_pattern` | prototypical-noun test set |
| `disambiguation_not_verbs` | `[string]` | `is_disambiguation_not_verb` | block spurious verb readings (bit, bus) |
| `morphology` | `Morphology{needs_e_ing,needs_e_ed,stemming_exceptions}` | `needs_e_ing`, `needs_e_ed`, `is_stemming_exception` | inflection/stemming edge cases |
| `units` | `{word: Dimension}` | `lookup_unit_dimension` | measure phrases (inch→Length) |
| `multi_word_expressions` | `MweEntry{pattern,lemma,pos,class?,features?}` | `build_mwe_trie` | compounds (fire engine→FireEngine) |
| `ontology` | `{part_whole[{whole,parts}],predicate_sorts{}}` | `get_possible_wholes`, `get_predicate_sort` | mereology + sort constraints |
| `axioms` | `{nouns{entails,hypernyms},adjectives{type},verbs{entails,manner}}` | `lookup_noun_entailments`, `lookup_noun_hypernyms`, `is_privative_adjective`, `lookup_verb_entailment` | meaning postulates (bachelor⊨unmarried) |

### FULL ENUMS (every variant, verbatim from `logicaffeine_lexicon/src/types.rs` unless noted)

- **VerbClass** (5) — verb `class`: `State`, `Activity` (default), `Accomplishment`,
  `Achievement`, `Semelfactive`.
- **Sort** (14) — noun `sort` / predicate sort: `Entity`, `Physical`, `Animate`, `Human`,
  `Plant`, `Place`, `Time`, `Abstract`, `Information`, `Event`, `Celestial`, `Value`, `Signal`,
  `Group`. (Subsumption: Human/Plant ⊆ Animate ⊆ Physical ⊆ Entity. `sort` may be omitted.)
- **Feature** (28) — `features` on nouns/verbs/adjectives:
  - verb transitivity: `Transitive`, `Intransitive`, `Ditransitive`
  - control/raising: `SubjectControl`, `ObjectControl`, `Raising`
  - verb semantics: `Opaque`, `Factive`, `Performative`, `Collective`, `Mixed`,
    `Distributive`, `Weather`, `Unaccusative`, `IntensionalPredicate`
  - noun: `Count`, `Mass`, `Proper`
  - gender: `Masculine`, `Feminine`, `Neuter`
  - animacy: `Animate`, `Inanimate`
  - adjective: `Intersective`, `NonIntersective`, `Subsective`, `Gradable`, `EventModifier`
- **Definiteness** (4) — `articles`: `Definite`, `Indefinite`, `Proximal`, `Distal`.
- **Time** (4) — `auxiliaries` / verb tense: `Past`, `Present`, `Future`, `None`.
- **Aspect** (3) — grammatical aspect: `Simple`, `Progressive`, `Perfect`.
- **Number** (2) — `pronouns` / agreement: `Singular`, `Plural`.
- **Gender** (4) — `pronouns` (note: spelled differently from the `Masculine/Feminine/Neuter`
  *Feature* names): `Male`, `Female`, `Neuter`, `Unknown`.
- **Case** (3) — `pronouns`: `Subject`, `Object`, `Possessive`.
- **Polarity** (2) — synonym/antonym canonical mappings: `Positive`, `Negative`.
- **Dimension** (5, `src/ast/logic.rs:49`) — `units`: `Length`, `Time`, `Weight`,
  `Temperature`, `Cardinality`.
- **PresupKind** (6, `src/token.rs:44`) — `presupposition_triggers`: `Stop`, `Start`,
  `Regret`, `Continue`, `Realize`, `Know`.
- **morphological_rules controlled strings** — `base_pos` ∈ {`Noun`, `Verb`}; `relation` ∈
  {`Practitioner`, `Agent`, `Patient`}. **MWE `pos`** ∈ {`Noun`, `Verb`, `Preposition`,
  `Conjunction`, `Quantifier`}.

**Idioms that aren't obvious from a single entry (these are what bite you):**
- **Latin `-s` singulars** (species, series): set `forms.plural` *equal to the singular* so the
  `-s`-stripping singularizer returns the word unchanged. A new `-s` singular noun mirrors this.
  (A noun with a *distinct* `-s` plural, e.g. genus/genera, is the one case the single
  `forms.plural` field can't fully capture — that is a real "the model can't map it" signal.)
- **Plural→lemma mapping** comes *only* from `forms.plural` (see `derive_irregular_plurals`):
  irregular plurals (mice, geese) are expressed there.
- **`features` drive FOL shape**: gender/animacy/control/intersectivity change the output, so a
  wrong feature mis-shapes the logic even when the word "compiles."

## 1. Get an article and triage it
- Fresh random page:  `./scripts/next-wiki.sh`
- Or reuse one:       `./scripts/triage-wiki.sh wikis/<slug>.txt`

Outputs land in `wikis/triage/<slug>/`. Read `verdict.json`, `worklist.md`, `clusters.json`.
(`fail` = doesn't compile; `partial` = compiles but lossy. Fix the **cluster/class**.)

## 2. Pick ONE bug (preference order)
1. A `parser_gap` cluster carrying an **`oracle`** — `oracle.expected_fol` is the spec.
2. An `actionable_lexicon_gap` cluster (gate `auto`) — the common, routine case.

Skip `needs_human.md` (genuine ambiguity, isolated noise). State which you picked and why.

## 3. Lexicon gaps are routine — here is the whole procedure

**Easiest path — use the tool: `./scripts/place-word.py --word <word>`.** It walks the
routing flowchart (type `?` at any prompt for a decision guide), validates enum values against
the crate, checks for homonyms/conflicts across every section, **warns about the companion
"second spots"** a word usually needs wired (see below), and — for a safe pure data-add —
**writes the entry into `lexicon.json` for you** (`--apply`, or it offers interactively). It
refuses to auto-write homonyms/conflicts/keywords (those need judgment). Then just `cargo
build` and re-triage. The manual steps below are the same thing by hand.

**Agent / non-interactive mode** (deterministic, no prompts): pass `--role <slug> --spec
'{...}' --apply --json`. It prints a machine-readable result and sets an **exit code**: `0`
applied/clean-plan, `1` blocked (conflict/homonym), `2` bad input (unknown role / invalid enum
/ missing field), `3` not-auto (keyword/special — needs Rust wiring), `5` apply failed. So the
loop can branch on the code instead of parsing prose. `--list-roles` prints the role slugs.
Example: `./scripts/place-word.py --word genus --role noun --spec '{"sort":"Group","plural":"genera"}' --apply --json`.

**`--enrich` (optional grounding)** surfaces real lexical data instead of guessing — plural &
verb forms (deterministic, reliable: `genus`→`genera`, `run`→`ran`), plus WordNet **candidate
sorts across senses** and hypernyms. Two honest caveats: `sort` is **sense-dependent** (WordNet
lists e.g. `genus`→Abstract *and* Group — pick by the sentence, don't trust the first), and it
needs optional deps (`pip install --user inflect lemminflect nltk` + `python -c "import nltk;
nltk.download('wordnet')"`; ~1.4 MB for the inflection libs, ~45 MB if you add WordNet). The
core tool has **zero hard deps** — without them, `--enrich` simply reports that no libs are
present and everything else still works.

**A word is rarely one JSON line — it often needs WIRING in a second spot.** The tool flags
these, but know them: a singular `-s` noun needs `forms.plural` (the singularizer in
`parser/mod.rs` else mangles it); a word that's already another POS becomes a homonym (the
lexer emits an `Ambiguous` token — `lexer.rs:2414` — changing existing parses; gate it with
`disambiguation_not_verbs`); a `keywords` entry needs a `TokenType` in `src/token.rs` **and** a
match arm in `build.rs::generate_lookup_keyword` (its `_ => continue` silently drops unmapped
keywords); focus particles / periphrastic modals / contractions / multi-word quantifiers /
block headers are **hardcoded in Rust**, not lexicon data (run the tool's "Special construct"
route for the file:line map). A bad `features` value is a **compile error**; a bad
`sort`/Dimension/keyword/PresupKind is **silently dropped**; a bad `class`/Definiteness
**silently defaults** — so validate (the tool does).

A `proposal.lexicon_entry` ships with a **suffix-guessed POS — verify it** (the harness gets
"now"→Noun, "up"→Noun wrong; those are an adverb and a particle). To verify: what role does
the word play *in the sentence*? Pick the section + enum values accordingly, mirror a sibling
entry already in `lexicon.json`, add it, `cargo build`, re-check the trace.

- Read the trace first: `wikis/triage/<slug>/sentences/NN.trace` (tokens+POS, AST, FOL,
  readings, socratic error). `localization.offending_text` is the token it tripped on — note it
  may not equal the cluster's headword.
- Edit `crates/logicaffeine_language/assets/lexicon.json`; mirror the closest existing entry.
- `cargo build` regenerates the tables. If a stale table is suspected, `touch` the JSON
  (and `build.rs` if you changed it) to force `build.rs` to rerun, then rebuild.
- Confirm with `compile("…")` or by re-tracing. **Then re-triage (step 6) — that is your proof.**

You do **not** need subagents for a normal word-add. Use an **Explore** agent only when you
must locate non-obvious machinery (e.g. which generated function a behavior flows through), and
a **Plan** agent only for a real parser/semantic change.

## 4. Tests: for behavior, NOT for vocabulary
- **Do not add a test for every missing word.** A word-add is data; the regen + re-triage is
  the verification. Per-word tests are bloat and will be rejected.
- **Do** add a NEW failing→passing test when you change *behavior*: a parser rule, a
  morphology/singularization rule, a codegen change in `build.rs`, or semantics. Name it for
  the **rule/phenomenon**, not the word (e.g. a `-s`-singular morphology test, not a "genus"
  test), and put it in `crates/logicaffeine_tests/tests/` near siblings for that phenomenon.
- **Never edit an existing test to make it pass.** If a green test must change, that is an
  escalation (step 5), not an edit. (See `CLAUDE.md` rule #4.)
- For a parser_gap with an oracle, the new test asserts the failing sentence compiles, ideally
  to FOL matching `oracle.expected_fol`.

## 5. When to actually escalate (the bar is high for vocabulary)

Proceed and fix it yourself for any ordinary word-add. **Escalate only if:**
- The word is **genuinely ambiguous in a way that changes the FOL** — multiple senses/POS and
  the sentence doesn't disambiguate (noun vs. verb with different meaning, etc.).
- It **conflicts** with an existing entry, or the fix would force **editing a green test**.
- The data model **genuinely cannot express it** — and even then, first ask whether a small,
  principled extension to the lexicon *machinery* (a struct field + `build.rs` codegen) is the
  right move. That is a real change (test it, keep the suite green, prefer human sign-off), but
  it is still "extend the lexicon," not "give up."
- The bug is real **semantics/parser** depth (scope, Kripke/modal, control, ellipsis) with no
  oracle, or subagents can't pin a clean root cause.

A missing common noun/adjective/verb with an obvious role is **none of these** — just fix it.

How to escalate: append a short entry to `wikis/triage/<slug>/needs_human.md` and report to
the human with the sentence, what you found, *why* it's a judgment call, and 2–3 concrete
options with a recommendation. Then wait.

## 6. Verify and confirm
- Run the **full suite** and confirm green (a failing test is a regression — fix before moving):
  `Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include" LIBRARY_PATH="/opt/homebrew/lib" cargo test --features verification --no-fail-fast > /tmp/test_all.txt 2>&1; echo "EXIT: $?" >> /tmp/test_all.txt`
  - The test crate pulls in Z3; you **must** pass the env vars above. Use `--features
    verification` consistently so the build hash matches and the lexicon actually recompiles.
- Re-triage the SAME page: `./scripts/triage-wiki.sh wikis/<slug>.txt`. Confirm the sentence
  left its old category (toward `clean`) and the verdict improved. Report what changed.

## 7. Close out — compact the context
When the job/loop iteration is **done** (fix landed + suite green + re-triaged, or a clean
escalation written), **run `/compact`** before finishing or before the next iteration. Each
iteration accumulates trace dumps, build logs, and subagent reports; compacting keeps the
working context lean so the next pass starts fresh and fast. Always compact at the end of the
loop/job — do not carry a bloated context into the next article.

## Guardrails (non-negotiable)
- The triage harness is **read-only**: it proposes, you apply (and only the safe items).
- Tests are **added for behavior, never per-word, never edited** to pass. Green before, green after.
- Don't touch `isolate_out_of_scope`. Fix clusters, not single lines. **Don't run git.**
- Escalate **contested meaning and real design forks** — not vocabulary.

---
