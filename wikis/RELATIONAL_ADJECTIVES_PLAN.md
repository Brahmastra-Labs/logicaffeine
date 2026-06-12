# Sprint plan — Relational adjectives (`Feature::Relational`)

## Goal & spec (the maximal sound form)

A relational/pertainymic adjective ("coastal" ← "coast", "dental" ← "tooth") is denominal and
**non-predicating**: "a coastal region" doesn't mean {coastal things} ∩ {regions}. We model it
as an **existential over the base noun + a named, underspecified, axiomatizable relation**:

```
a coastal region   →   Region(x) ∧ ∃y ( Coast(y) ∧ Pertains(x, y) )
```

The relation defaults to `Pertains` and is **overridable per adjective** (e.g. `coastal` could
carry `Near`). The base noun and relation live in the lexicon entry:

```jsonc
{ "lemma": "Coastal", "features": ["Relational"],
  "relational": { "base": "Coast", "relation": "Pertains" } }
```

This subsumes the weaker options (drop the ∃ → term-level `Pertains(x,Coast)`; name the relation
`Near` → specific; ignore it → flat `Coastal(x)`) and is extensible *down* via axioms — you can
always specialize, never have to throw structure away.

## Audit findings (captured from the live compiler — ground truth)

- **Notation:** `∃` / `∀` / `∧` / `→`; predicates Capitalized `Coast(x)`; vars `x`, `y`, event `e`.
- **Adjective→FOL site:** `crates/logicaffeine_language/src/parser/quantifier.rs:1494–1517`
  (indefinite) and `:1584–1607` (definite). It already branches on `is_subsective(adj)` to emit a
  2-arg `Adj(x, ^Noun)` vs 1-arg `Adj(x)`. **This is the hook** — add an `is_relational_adjective`
  branch that emits the existential.
- **Reusable existential machinery:** the same module builds `LogicExpr::Quantifier{ kind:
  Existential, variable, body }` and allocates fresh vars (`next_var_name`) — reuse it for the
  `∃y`.
- **⚠ PRE-EXISTING BUG (prerequisite):** indefinite subjects drop adjectives. `"A red car is
  fast."` → `∃x((Car(x) ∧ Vehicle(x)) ∧ Fast(x))` (Red dropped); the antecedent/indefinite path
  (`parser/clause.rs:742–778`) ignores `np.adjectives`. Universal subjects keep them. **Therefore
  Phase 1 tests use universal contexts** to isolate the relational feature; fixing the indefinite
  drop is **Phase 0**, sequenced separately (it changes existing indefinite FOL → regression risk).
- **Schema mirror:** the noun `derivation` pattern is the exact template — `NounDerivation` struct
  + `lookup_noun_derivation` (`build.rs:~97/1607`). We mirror it for adjectives.

## Implementation phases

**Phase 1 — classification + relational FOL in universal contexts (decoupled, lowest risk):**
1. `crates/logicaffeine_lexicon/src/types.rs`: add `Feature::Relational` (after `EventModifier`,
   ~462) and its `Feature::from_str` arm (~499). (No exhaustive `Feature` matches exist to break.)
2. `crates/logicaffeine_language/build.rs`:
   - `struct AdjectiveRelational { base, relation }` + `relational: Option<AdjectiveRelational>`
     on `AdjectiveDefinition` (~117).
   - carry it through `AdjectiveDbEntry` + `expand_adjectives_to_db_entries` (~1128).
   - generate `lookup_relational_adjective(word) -> Option<(&str,&str)>` and
     `is_relational_adjective` (mirror `lookup_noun_derivation` / `generate_is_check`).
3. `crates/logicaffeine_language/assets/lexicon.json`: add the relational adjectives the RED tests
   use (`Coastal`, `Dental`, `Nuclear`, `Marine`, …) with `relational{base,relation}`. **Use
   `./scripts/place-word.py --enrich` to pull the base noun from WordNet pertainyms.**
4. `parser/quantifier.rs:~1505`: add the `is_relational_adjective` branch — look up `(base,
   relation)`, allocate fresh `y`, emit `∃y( Base(y) ∧ Relation(x, y) )`, AND it into the
   restriction. Use the lexicon **`relation`** (not the adjective name).
5. RED test `crates/logicaffeine_tests/tests/phase_relational_adjectives.rs` (Phase-1 sentences).
6. `cargo build` (regen lexicon) → RED → GREEN → **full suite green**
   (`--features verification`).

**Phase 0 — fix indefinite adjective drop (prerequisite for indefinite-context tests):**
- `parser/clause.rs:742–778` (and the indefinite NP path): emit `np.adjectives` predicates when
  building the type restriction, routing through the SAME relational/subsective branch as Phase 1
  (so relational works in `∃` contexts too). **Regression risk:** this adds adjective predicates
  to existing indefinite FOL — run the full suite, expect some green tests to need review (if a
  green test must change, that's an escalation, not an edit).

**Phase 2 — relation override + axiom specialization:** prove a per-adjective `relation` override
(e.g. `Near`) and that a domain axiom can specialize `Pertains`.

**Phase 3 — regression guards:** intersective ("red") and subsective ("large") adjectives are
UNCHANGED.

## RED-TEST SENTENCES (the spec — what we'll newly handle)

Tests use `compile_forest(...)` and assert the FOL `.contains(...)` the listed predicates
(house style). Expected FOL is in the captured notation.

### Phase 1 — relational FOL, universal contexts (testable once Phase 1 lands)

| # | Sentence | Expected FOL (target) | Asserts |
|---|---|---|---|
| 1 | Every coastal region is wet. | `∀x(((∃y(Coast(y) ∧ Pertains(x,y)) ∧ Region(x)) → Wet(x)))` | `Coast(`, `Pertains(`, `Region(`, `Wet(`; **NOT** bare `Coastal(x)` |
| 2 | Every dental procedure is expensive. | `∀x(((∃y(Tooth(y) ∧ Pertains(x,y)) ∧ Procedure(x)) → Expensive(x)))` | `Tooth(`, `Pertains(` |
| 3 | Every nuclear reactor is dangerous. | `∀x(((∃y(Nucleus(y) ∧ Pertains(x,y)) ∧ Reactor(x)) → Dangerous(x)))` | `Nucleus(`, `Pertains(` |
| 4 | Every marine animal swims. | `∀x((∃y(Sea(y) ∧ Pertains(x,y)) ∧ Animal(x)) → ∃e(Swim(e) ∧ Agent(e,x)))` | `Sea(`, `Pertains(`, `Animal(` |
| 5 | Every postal worker is busy. | `∀x(((∃y(Post(y) ∧ Pertains(x,y)) ∧ Worker(x)) → Busy(x)))` | `Post(`, `Pertains(` |

### Phase 2 — relation override + specialization

| # | Sentence | Expected FOL | Asserts |
|---|---|---|---|
| 6 | Every coastal town is small. (coastal carries `relation:"Near"`) | `∀x(((∃y(Coast(y) ∧ Near(x,y)) ∧ Town(x)) → Small(x)))` | `Near(`, **NOT** `Pertains(` (override works) |
| 7 | Every solar panel is efficient. | `∀x(((∃y(Sun(y) ∧ Pertains(x,y)) ∧ Panel(x)) → Efficient(x)))` | `Sun(`, `Pertains(` |

### Phase 3 — regression guards (these must stay UNCHANGED)

| # | Sentence | Expected FOL (unchanged) | Asserts |
|---|---|---|---|
| 8 | Every red car is fast. | `∀x(((Red(x) ∧ Car(x)) → Fast(x)))` | `Red(x)` 1-arg; **NO** `∃y`, **NO** `Pertains` |
| 9 | Every large mouse is quiet. | subsective `Large(x, ^Mouse)` preserved | `Large(` 2-arg; no relational expansion |

### Phase 0 + Phase 4 — indefinite contexts (need the Phase-0 drop fix first)

| # | Sentence | Expected FOL | Notes |
|---|---|---|---|
| 10 | A coastal region is wet. | `∃x((Region(x) ∧ ∃y(Coast(y) ∧ Pertains(x,y)) ∧ Wet(x)))` | currently drops the adj — Phase 0 |
| 11 | A red car is fast. | `∃x((Car(x) ∧ Red(x) ∧ Fast(x)))` | Phase-0 regression target (Red currently dropped) |
| 12 | It is native to coastal regions of Japan. | relational `coastal` inside a PP | the real Wikipedia motivator (nipponanthemum); stretch goal |

## Open decisions / risks
- **Instance vs kind level:** `∃y` binds an instance coast; a few relational adjectives are
  kind-level ("nuclear energy" ≈ nuclei-in-general). We default to the existential (most
  expressive); add a `kind: true` flag on `relational` later if a word needs it.
- **Default relation name:** `Pertains` (uniform). Confirm the predicate name fits the FOL house
  style before locking the RED tests.
- **Phase 0 blast radius:** adding adjective predicates to indefinite subjects may change other
  green tests' expected FOL — handle as escalation if so, don't silently edit tests.
- **Base-noun normalization:** WordNet pertainym lemmas are clean here (coast, tooth) but verify
  each maps to an existing/added noun lemma.
