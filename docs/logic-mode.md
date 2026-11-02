# Logic mode — English → First-Order Logic

English lets one sentence mean two things and never says which: *every woman loves a man* can mean
one man loved by all of them, or a man for each. Logic mode parses English into First-Order Logic,
where each meaning is its own formula a machine can check. It is deterministic: where a sentence is
genuinely ambiguous, LOGOS can return *every* reading rather than guessing one. It also parses
*strictly*: a sentence whose words can't all be consumed fails with a `TrailingTokens` error rather
than yielding a silent partial reading, so dropped meaning is a bug, not a style nit.

After this guide you can call the API, read the shape of its output, and find the phase test that
pins any construction. Every example below is produced by the compiler itself.

```rust
let fol = logicaffeine_language::compile("Every man is mortal.")?;
// → "∀x((Man(x) → Mortal(x)))"
```

## The API

| Function | Signature | Notes |
|----------|-----------|-------|
| `compile` | `(&str) -> Result<String, ParseError>` | one reading |
| `compile_forest` | `(&str) -> Vec<String>` | every reading (infallible), up to `MAX_FOREST_READINGS` |
| `compile_all_scopes` | `(&str) -> Result<Vec<String>, ParseError>` | the distinct quantifier-scope readings |
| `compile_with_options` | `(&str, CompileOptions) -> Result<String, ParseError>` | choose the output format / pragmatics |

Output formats (`OutputFormat`): **Unicode** (default), **LaTeX**, **SimpleFOL**, and **Kripke**
(modals lowered to explicit world quantification). `compile_pragmatic` adds scalar implicatures as
a separate, cancellable line. From the terminal, [`largo logic`](cli.md#logic) takes the sentence
inline, from `--file`, or from stdin, with `--format unicode|latex|ascii|kripke`,
`--all-readings`, `--pragmatic` and `--discourse` (one sentence per line, shared anaphora).

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
proportional (`most`, `few`, `many`), counted (`three`, `at least three`, `at most three`),
infinitary and generic readings are all handled (`QuantifierKind` in `ast/logic.rs`). A determiner
reads the same way in every construction it heads: it fixes the quantifier and how the restrictor
meets the scope — `R → S` under `all` and `all but finitely many`, `R → ¬S` under `no`, `R ∧ S`
under the rest.

### Infinitary quantifiers

`infinitely many` is Mostowski's quantifier ∃^∞, `finitely many` (or `only finitely many`) is
∃^<∞, and `all but finitely many` is the cofinite quantifier ∀^∞:

```
"Infinitely many primes are odd."
→ ∃^∞x((Primes(x) ∧ Odd(x)))

"All but finitely many dogs bark."
→ ∀^∞x((Dogs(x) → ∃e(Bark(e) ∧ Agent(e, x))))
```

LaTeX writes them `\exists^{\infty}`, `\exists^{<\infty}` and `\forall^{\infty}`. The proof
engine reads them over the order of the naturals, where a set is infinite exactly when it is
unbounded: ∃^∞x φ is ∀b ∃x (x > b ∧ φ), ∀^∞x φ is ∃b ∀x (x > b → φ), and ∃^<∞x φ is
∃b ∀x (x > b → ¬φ).

`infinitely often` is G F, at either end of the sentence, and `eventually, always` is its dual F G:

```
"The light flashes infinitely often."
→ G(F(∃x(((Light(x) ∧ ∀y((Light(y) → y = x))) ∧ HAB(∃e(Flash(e) ∧ Agent(e, x)))))))
```

`x is infinitely close to y` is the relation `InfinitelyClose(x, y)` of nonstandard analysis.

## Connectives & negation

`and` (∧), `or` (∨), `if…then` (→), `iff` (↔), and `not` (¬). `and` binds tighter than `or`.

```
"John does not run."
→ ¬HAB(∃e(Run(e) ∧ Agent(e, John)))
```

(`HAB` is the habitual-aspect operator; see Tense & aspect below.)

## Modality

Modal verbs lower to Kripke-style modal operators with a force value (`□` necessity, `◇`
possibility), over alethic / deontic / temporal domains.

```
"Every student must study."
→ ∀x((Student(x) → □_{1.0} ∃e(Study(e) ∧ Agent(e, x))))
```

Use `OutputFormat::Kripke` to lower modals to explicit world quantification.

## Tense & aspect

Past/future tense and progressive / perfect / habitual / iterative aspect are tracked (Prior-style
operators plus LTL operators for hardware temporal logic). `"John loved Mary."` carries `Past(e)`;
present-tense generics carry `HAB`.

## Comparatives

```
"John is taller than Mary."
→ Taller(John, Mary)
```

Degree semantics covers comparatives, superlatives and equatives.

## Relative clauses

```
"The man who owns a dog is happy."
→ ∃x(((Man(x) ∧ (((Dog(y) ∧ Animal(y)) ∧ Mammal(y)) ∧ ∃e(Own(e) ∧ Agent(e, x) ∧ Theme(e, y)))) ∧ Happy(x)))
```

## Arithmetic sentences

A quantified arithmetic sentence has its own term layer
([`parser/math.rs`](../crates/logicaffeine_language/src/parser/math.rs)): `plus`, `minus`, `times`
and `divided by` are `Add`, `Sub`, `Mul` and `Div`, a lowercase-leading name is a variable (bound
when the `For all` prefix names it), an uppercase- or digit-leading name is a constant, a number
word is a numeral in any case, and `f(n)` is a function term.

```
"For all n, n plus 0 equals n."
→ ∀n(Add(n, 0) = n)

"For all a b, a plus b equals b plus a."
→ ∀a(∀b(Add(a, b) = Add(b, a)))

"For all but finitely many n, n plus 0 equals n."
→ ∀^∞n(Add(n, 0) = n)
```

A binder outranks the numeral table (`For all one, one times one equals one.` binds `one`), and a
numeral too large for `i64` travels as its own spelling rather than a rounded float. These are the
sentences a `## Theorem` proves with the ring oracle; see
[Proof & verification](proof-and-verification.md#arithmetic-in-english).

## Reciprocals & plurals

A reciprocal is symmetric, so it is genuinely ambiguous — `compile` returns *both* numbered readings
rather than guessing:

```
"John and Mary love each other."
→ 1) Love(John, Mary)  2) Love(Mary, John)
```

Plurals follow the verb: a distributive verb (*sleep*) applies to each member, a collective one
(*gather*) to the group, and a mixed one (*lift*) yields both readings from `compile_forest`.

## Scope ambiguity

When a sentence has more than one reading, `compile_forest` / `compile_all_scopes` return them all
instead of committing to one. This is the determinism guarantee in practice.

```rust
let readings = logicaffeine_language::compile_forest("Every man loves a woman.");
// → surface scope (∀ > ∃) and inverse scope (∃ > ∀); compile_forest is infallible
```

Here are both readings of the sentence this guide opened with, exactly as `compile_all_scopes` returns them
(and as `largo logic --all-readings "Every woman loves a man."` prints them):

```
"Every woman loves a man."
1. ∀x((Woman(x) → ∃y((Man(y) ∧ ∃e(Love(e) ∧ Agent(e, x) ∧ Theme(e, y))))))
2. ∃y((Man(y) ∧ ∀x((Woman(x) → ∃e(Love(e) ∧ Agent(e, x) ∧ Theme(e, y))))))
```

Reading 1 lets each woman love a different man; reading 2 says there is one man every woman loves. Neither is
the "right" one, so neither is dropped.

## The long tail

LOGOS covers a wide range of linguistic phenomena, each pinned by a phase test in
[`crates/logicaffeine_tests/tests/`](../crates/logicaffeine_tests/tests/):

| Phenomenon | Test file |
|------------|-----------|
| Garden-path sentences | `phase1_garden_path` |
| Wh-questions | `phase5_wh_movement`, `phase_question_answer` |
| Ellipsis / sluicing / gapping | `phase10_ellipsis`, `phase10b_sluicing`, `phase132_gapping` |
| Event semantics & thematic roles | `phase65_event_semantics`, `phase41_event_adjectives` |
| Higher-order / λ-calculus (Montague) | `phase66_higher_order` |
| Presupposition projection | `phase113_presupposition_projection`, `phase142_vandersandt` |
| Counterfactual conditionals | `phase110_counterfactuals` |
| Binding & coreference | `phase114_binding_theory`, `phase143_binding` |
| Scalar implicature & focus | `phase130_implicature`, `phase120_clefts` |
| Causality / concession | `phase122_causal_concessive` |
| Mass vs count | `phase115_mass_count` |
| Metaphor / metonymy / deixis / vagueness | `phase11_metaphor`, `phase129_metonymy`, `phase128_deixis`, `phase127_vagueness` |
| Discourse / anaphora (multi-sentence) | `phase42_drs`, `phase45_session` (the `Session` API) |

The test suite is the living specification: when in doubt about what a construction yields, the
phase test shows the exact output.

## See also

- The implementation crate → [`logicaffeine_language`](../crates/logicaffeine_language/README.md)
- The proof side (what you can *do* with the logic) → [Proof & verification](proof-and-verification.md)

## Where this is tested

The code is in [`crates/logicaffeine_language/`](../crates/logicaffeine_language/): the lexer
([`lexer.rs`](../crates/logicaffeine_language/src/lexer.rs)), the parser modules
([`parser/`](../crates/logicaffeine_language/src/parser/)), the AST
([`ast/logic.rs`](../crates/logicaffeine_language/src/ast/logic.rs)), and the transpiler
([`transpile.rs`](../crates/logicaffeine_language/src/transpile.rs)). Every `"English" → FOL` pair
in this guide is checked against the live compiler by `doc_examples`.

Beyond the long-tail table above, each section's phenomena are held by:

- Quantifiers: `phase20_axioms`, `phase112_partitive_quantifiers`, `phase117_cumulative`,
  `logic_determiner_readings`, `logic_infinitary_quantifiers`.
- Connectives & negation: `phase2_polarity`, `phase15_negation`.
- Modality: `phase139_modal_frames`, `phase123_evidentials`, `phase125_optatives`.
- Tense & aspect: `phase3_time`, `phase3_aspect`, `phase16_aspect`, `phase_hw_temporal`.
- Comparatives: `phase8_degrees`, `phase17_degrees`, `phase119_equatives`, `phase126_degree_standard`.
- Relative clauses: `phase5_wh_movement`, `phase135_appositive_rc`.
- Arithmetic sentences: `L_language_extra`.
- Reciprocals & plurals: `phase4_reciprocals`, `phase18_plurality`, `phase19_group_plurals`,
  `phase44_distributive`.
- Scope ambiguity: `phase12_ambiguity`, `phase116_island_scope`.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)
