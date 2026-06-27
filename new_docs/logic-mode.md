# Logic mode — English → First-Order Logic

Logic mode parses an English sentence and emits formal logic. It is deterministic: where a sentence
is genuinely ambiguous, LOGOS can return *every* reading rather than guessing one. It also parses
*strictly* — a sentence whose words can't all be consumed fails with a `TrailingTokens` error rather
than yielding a silent partial reading, so dropped meaning is a bug, not a style nit.

```rust
let fol = logicaffeine_language::compile("Every man is mortal.")?;
// → "∀x((Man(x) → Mortal(x)))"
```

Source of truth: [`crates/logicaffeine_language/`](../crates/logicaffeine_language/) — the lexer
([`lexer.rs`](../crates/logicaffeine_language/src/lexer.rs)), the parser modules
([`parser/`](../crates/logicaffeine_language/src/parser/)), the AST
([`ast/logic.rs`](../crates/logicaffeine_language/src/ast/logic.rs)), and the transpiler
([`transpile.rs`](../crates/logicaffeine_language/src/transpile.rs)). Every example below is produced
by the actual compiler.

## The API

| Function | Signature | Notes |
|----------|-----------|-------|
| `compile` | `(&str) -> Result<String, ParseError>` | one reading |
| `compile_forest` | `(&str) -> Vec<String>` | every reading (infallible), up to `MAX_FOREST_READINGS` |
| `compile_all_scopes` | `(&str) -> Result<Vec<String>, ParseError>` | the distinct quantifier-scope readings |
| `compile_with_options` | `(&str, CompileOptions) -> Result<String, ParseError>` | choose the output format / pragmatics |

Output formats (`OutputFormat`): **Unicode** (default), **LaTeX**, **SimpleFOL**, and **Kripke**
(modals lowered to explicit world quantification).

## The shape of the output

LOGOS does not stop at surface predicates. Two features show up throughout:

- **Neo-Davidsonian event semantics** — verbs introduce an event variable `e` with explicit
  thematic roles (`Agent`, `Theme`, `Past`, …):

  ```
  "John loved Mary."
  → ∃e(Love(e) ∧ Agent(e, John) ∧ Theme(e, Mary) ∧ Past(e))
  ```

- **Ontology expansion** — nouns carry their semantic class, so a *dog* is also an *animal* and a
  *mammal*:

  ```
  "Every dog barks."
  → ∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ∃e(Bark(e) ∧ Agent(e, x))))
  ```

## Quantifiers

```
"Some cat sleeps."
→ ∃x((((Cat(x) ∧ Animal(x)) ∧ Mammal(x)) ∧ ∃e(Sleep(e) ∧ Agent(e, x))))

"No bird flies."
→ ∀x(((Bird(x) ∧ Animal(x)) → ¬∃e(Fly(e) ∧ Agent(e, x))))
```

Universal (`every`, `all`, `each`), existential (`some`, `a`, `an`), negative (`no`, `none`),
cardinals, and generic readings are all handled (`QuantifierKind` in `ast/logic.rs`; see the
`phase20_axioms`, `phase112_partitive_quantifiers`, `phase117_cumulative` tests).

## Connectives & negation

`and` (∧), `or` (∨), `if…then` (→), `iff` (↔), and `not` (¬). `and` binds tighter than `or`.

```
"John does not run."
→ ¬HAB(∃e(Run(e) ∧ Agent(e, John)))
```

(`HAB` is the habitual-aspect operator — see below.) Tests: `phase2_polarity`, `phase15_negation`.

## Modality

Modal verbs lower to Kripke-style modal operators with a force value (`□` necessity, `◇`
possibility), over alethic / deontic / temporal domains.

```
"Every student must study."
→ ∀x((Student(x) → □_{1.0} ∃e(Study(e) ∧ Agent(e, x))))
```

Use `OutputFormat::Kripke` to lower modals to explicit world quantification. Tests:
`phase139_modal_frames`, `phase123_evidentials`, `phase125_optatives`.

## Tense & aspect

Past/future tense and progressive / perfect / habitual / iterative aspect are tracked (Prior-style
operators plus LTL operators for hardware temporal logic). `"John loved Mary."` carries `Past(e)`;
present-tense generics carry `HAB`. Tests: `phase3_time`, `phase3_aspect`, `phase16_aspect`,
`phase_hw_temporal`.

## Comparatives

```
"John is taller than Mary."
→ Taller(John, Mary)
```

Degree semantics with comparatives and superlatives: `phase8_degrees`, `phase17_degrees`,
`phase119_equatives`, `phase126_degree_standard`.

## Relative clauses

```
"The man who owns a dog is happy."
→ ∃x(((Man(x) ∧ (((Dog(y) ∧ Animal(y)) ∧ Mammal(y)) ∧ ∃e(Own(e) ∧ Agent(e, x) ∧ Theme(e, y)))) ∧ Happy(x)))
```

Tests: `phase5_wh_movement`, `phase135_appositive_rc`.

## Reciprocals & plurals

A reciprocal is symmetric, so it is genuinely ambiguous — `compile` returns *both* numbered readings
rather than guessing:

```
"John and Mary love each other."
→ 1) Love(John, Mary)  2) Love(Mary, John)
```

Reciprocals (`phase4_reciprocals`), collective vs distributive plurals (`phase18_plurality`,
`phase19_group_plurals`, `phase44_distributive`).

## Scope ambiguity

When a sentence has more than one reading, `compile_forest` / `compile_all_scopes` return them all
instead of committing to one — this is the determinism guarantee in practice.

```rust
let readings = logicaffeine_language::compile_forest("Every man loves a woman.");
// → surface scope (∀ > ∃) and inverse scope (∃ > ∀); compile_forest is infallible
```

Tests: `phase12_ambiguity`, `phase116_island_scope`.

## The long tail

LOGOS covers a wide range of linguistic phenomena, each pinned by a phase test in
[`crates/logicaffeine_tests/tests/`](../crates/logicaffeine_tests/tests/):

| Phenomenon | Test file |
|------------|-----------|
| Garden-path sentences | `phase1_garden_path` |
| Wh-questions | `phase5_wh_movement`, `question.rs` |
| Ellipsis / sluicing / gapping | `phase10_ellipsis`, `phase10b_sluicing`, `phase132_gapping` |
| Event semantics & thematic roles | `phase65_event_semantics`, `phase41_event_adjectives` |
| Higher-order / λ-calculus (Montague) | `phase66_higher_order` |
| Presupposition projection | `phase113_presupposition_projection`, `phase142_vandersandt` |
| Counterfactual conditionals | `phase110_counterfactuals` |
| Binding & coreference | `phase114_binding_theory`, `phase143_binding` |
| Scalar implicature & focus | `phase130_implicature`, `pragmatics.rs` |
| Causality / concession | `phase122_causal_concessive` |
| Mass vs count | `phase115_mass_count` |
| Metaphor / metonymy / deixis / vagueness | `phase11_metaphor`, `phase129_metonymy`, `phase128_deixis`, `phase127_vagueness` |
| Discourse / anaphora (multi-sentence) | `phase42_drs`, `phase45_session` (the `Session` API) |

The test suite is the living specification — when in doubt about what a construction yields, the
phase test shows the exact output.

## See also

- The implementation crate → [`logicaffeine_language`](../crates/logicaffeine_language/README.md)
- The proof side (what you can *do* with the logic) → [Proof & verification](proof-and-verification.md)

---
[Docs index](README.md) · [Root README](../NEW_README.md) · [Changelog](../CHANGELOG.md)
