# LOGOS Logical Architecture & Semantics

> "Logic is the hygiene the mathematician practices to keep his ideas healthy and strong." — Hermann Weyl

This document outlines the **Logical Mode** of the LOGOS system. Unlike the standard compilation pipeline (which transpiles English to Rust), the Logical Mode functions as a **Semantic Engine** and **Model Checker**, transforming natural language into rigorous formalisms for analysis and verification.

---

## 1. System Overview: The Dual-Mode Engine

LOGOS operates in two distinct modes sharing a common parser:

1.  **Imperative Mode (The Language):** Transpiles English prose into executable Rust code (`structs`, `enums`, control flow).
2.  **Logical Mode (The Engine):** Transpiles English prose into **First-Order Logic (FOL)** and **Kripke Structures** for semantic analysis and Z3 verification.

This document focuses exclusively on **Mode 2**.

---

## 2. Core Logic Capabilities

The LOGOS AST (`src/ast/logic.rs`) supports a superset of standard First-Order Logic, enriched with linguistic semantics.

### 2.1 Quantifiers & Connectives
We support full FOL with extended generalized quantifiers:

| Feature | Supported | Implementation |
| :--- | :--- | :--- |
| **Standard** | `∀`, `∃` | `QuantifierKind::Universal`, `QuantifierKind::Existential` |
| **Generalized** | `Most`, `Few`, `Many` | Semantic handling for non-standard quantifiers |
| **Numeric** | `AtLeast(n)`, `AtMost(n)` | Cardinality constraints |
| **Generic** | `Gen` | Law-like generalizations ("Birds fly") |

### 2.2 Event Semantics (Neo-Davidsonian)
Verbs are treated as predicates over events (`e`), allowing for precise handling of adverbs and thematic roles.

*   **Input:** "John kicked the ball quickly."
*   **Logical Form:** `∃e(Kick(e) ∧ Agent(e, John) ∧ Theme(e, Ball) ∧ Manner(e, Quickly))`

### 2.3 Tense & Aspect (Reichenbach)
Time is modeled using Reichenbach's three-point system: **Event (E)**, **Reference (R)**, and **Speech (S)**.

*   **Past Perfect:** `E < R < S` ("had run")
*   **Future Perfect:** `S < R, E < R` ("will have run")

### 2.4 Mereology & Plurals (Link-style)
Plural subjects are handled using Link's semi-lattice mereology, allowing for the distinction between **Collective** and **Distributive** readings.

*   **Sigma Operator (`σ`):** Used for maximal sums ("The dogs" → `σx.Dog(x)`).
*   **Distributive Operator (`*`):** Wraps predicates for individual readings.
*   **Group Quantifier:** Handles cardinal indefinites with collective readings ("Two boys lifted a rock").
    *   *Collective:* `∃g(Group(g) ∧ Count(g, 2) ∧ Lift(g, Rock))`
    *   *Distributive:* `∃x∃y(Boy(x) ∧ Boy(y) ∧ x≠y ∧ Lift(x, Rock) ∧ Lift(y, Rock))`

---

## 3. Deep Mode: Kripke Semantics & Modality

The "Deep Mode" pipeline (`src/semantics/kripke.rs`) lowers surface-level modal operators into **explicit quantification over possible worlds**. This is the system's answer to modal logic paradoxes.

### 3.1 The Lowering Process
Modal operators are transformed into First-Order constraints on a world set `W` with accessibility relations `R`.

*   **Possibility (`◇P`):** "John can fly."
    *   *Lowers to:* `∃w'(Accessible_Alethic(w₀, w') ∧ Fly(John, w'))`
*   **Necessity (`□P`):** "John must fly."
    *   *Lowers to:* `∀w'(Accessible_Alethic(w₀, w') → Fly(John, w'))`

### 3.2 Modal Domains
The system distinguishes between different "flavors" of modality, each generating distinct accessibility predicates:

*   **Alethic:** `Accessible_Alethic` (Logical/Physical possibility)
*   **Deontic:** `Accessible_Deontic` (Obligation/Permission)
*   **Epistemic:** `Accessible_Epistemic` (Knowledge/Belief)

---

## 4. Discourse Representation Theory (DRT)

The system implements a version of Kamp's **Discourse Representation Theory** (`src/drs.rs`) to handle the "memory" of natural language discourse.

### 4.1 Donkey Anaphora & Force
Unlike standard FOL, DRT allows indefinites to change their quantificational force based on their structural position (Discourse Boxes).
*   **Indefinite in Main Clause:** Existential force (`∃`).
*   **Indefinite in "If" clause:** Universal force (`∀`).
    *   *Example:* "If a farmer owns a donkey, he beats it."
    *   *Logic:* `∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x,y)) → Beat(x,y))`

### 4.2 Accessibility Constraints
DRT provides the rules for when a pronoun can bind to an antecedent:
*   **Negation & Disjunction:** Create "opaque" boxes that block outward accessibility.
*   **Conditionals:** The consequent can "see" referents introduced in the antecedent, but not vice versa.

---

## 5. The Axiom Layer (Meaning Postulates)

LOGOS features a post-parse transformation pass (`src/semantics/axioms.rs`) that expands predicates using **Analytic Entailments** and **Lexical Decomposition**.

*   **Canonical Resolution:** `Lack(x, y)` is automatically expanded to `¬Have(x, y)`.
*   **Privative Adjectives:** Handles "fake", "former", "alleged".
    *   *Example:* `Fake-Gun(x)` expands to `¬Gun(x) ∧ Resembles(x, ^Gun)`.
*   **Hypernym Chains:** `Dog(x)` can be automatically enriched with `Animal(x)` for downstream inference.
*   **Analytic Expansion:** `Bachelor(x)` expands to `Unmarried(x) ∧ Male(x) ∧ Adult(x)`.

---

## 6. Advanced Linguistic Logic Features

We support several "higher-order" features required for full English comprehension.

### 6.1 The Lambda Calculus Backbone
The engine uses **Lambda Calculus** (`src/lambda.rs`) as the glue for compositional semantics, enabling high-level operations beyond standard FOL.

*   **Abstraction (`λx.E`):** Used for question semantics ("Who runs?" → `λx.Run(x)`) and refinement types.
*   **Application (`(f x)`):** Allows for higher-order predicates.
*   **Type Lifting:** Proper names like "John" can be lifted into quantifiers (`λP.P(j)`) to solve scope ambiguities uniformly (e.g., "John loves every woman" vs "Every woman loves John").

### 6.2 Structural Movement & Control
We handle complex syntactic transformations where the logical subject differs from the surface subject.

*   **Control Theory:**
    *   *Subject Control:* "John wants to leave." (John is the leaver).
    *   *Object Control:* "John persuaded Mary to go." (Mary is the goer).
*   **Raising:** "John seems to be happy." (John is logically the subject of "happy", not "seems").
*   **Wh-Movement:** Handles long-distance dependencies across clause boundaries.
    *   *Input:* "Who did John say Mary loves?"
    *   *Logic:* `λx.Say(John, Love(Mary, x))`

### 6.3 Pragmatics & Interrogatives
The system analyzes **Illocutionary Force**, not just truth conditions.

*   **Question Semantics:**
    *   *Wh-Questions:* Lambda abstractions (`?x.P(x)`).
    *   *Yes/No Questions:* Propositional queries (`?P`).
    *   *Sluicing:* Reconstructs missing content from context ("Someone left. I know who [left].").
*   **Speech Acts:**
    *   *Performatives:* "I promise to go" → `SpeechAct(Promise, I, Go(I))`.
    *   *Indirect Imperatives:* "Can you pass the salt?" transforms from a Question into a Command `Imperative(Pass(You, Salt))` based on agent capability.

### 6.4 Discourse Context & Anaphora
We maintain a persistent **Discourse Context** to track narrative progression.

*   **Narrative Time:** Events in a sequence are automatically constrained by `Precedes(e_n, e_{n+1})`.
    *   *Input:* "John ran. Mary laughed."
    *   *Logic:* `∃e₁(Run...) ∧ ∃e₂(Laugh...) ∧ Precedes(e₁, e₂)`
*   **Cross-Sentence Anaphora:** Resolves pronouns ("He", "She") against a history of gender/number-tagged entities.
*   **VP Ellipsis:** Caches the last event template to reconstruct fragments like "Mary does too."

### 6.5 Ambiguity & Parse Forests
Unlike simple scope enumeration, our **Parse Forest** generates multiple distinct syntactic trees for structural ambiguities.

*   **Structural:** "I saw the man with the telescope" (Instrument vs Modifier attachment).
*   **Lexical:** "I saw her duck" (Noun vs Verb).
*   **Safety:** We cap the forest at `MAX_FOREST_READINGS` (12) to prevent combinatorial explosion.

### 6.6 Graded Modality (Modal Vectors)
Modals are not binary operators but continuous **Vectors** `{ domain, force }`.

*   **Force Mapping:**
    *   `1.0` (Must/Necessity)
    *   `0.9` (Shall)
    *   `0.6` (Should/Ought)
    *   `0.5` (Can/May)
    *   `0.0` (Cannot)
*   **Kripke Integration:** High force maps to `∀w`, low force to `∃w`.

### 6.7 Presupposition & Focus
*   **Presupposition:** "John stopped smoking" → `Assertion: ¬Smoke(J) | Presup: Smoke(J)`.
*   **Focus (Rooth):** "Only John ran" → `ONLY(John, λx.Ran(x))` (Assertion: John ran; Alternatives: No one else ran).

### 6.8 Intensionality (De Re / De Dicto)
*   **De Re:** `∃x(Unicorn(x) ∧ Seek(John, x))` (Specific unicorn).
*   **De Dicto:** `Seek(John, ^Unicorn)` (Concept of a unicorn).

---

## 7. Verification: The Z3 Integration

LOGOS integrates the **Z3 SMT Solver** to provide **Semantic Verification** (`⊨`).

### 7.1 How it Works
The `logos_verification` crate maps the LOGOS AST into Z3's internal representation:
*   **Booleans/Integers:** Mapped directly to Z3 sorts.
*   **Entities:** Mapped to an uninterpreted `Object` sort.
*   **Predicates:** Mapped to uninterpreted functions (e.g., `Fly: Object -> Bool`).

### 7.2 Capabilities
*   **Consistency Checking:** "Is this set of sentences satisfiable?"
*   **Entailment (via Refutation):** To prove `A ⊨ B`, the system checks if `A ∧ ¬B` is **UNSAT**.
*   **Arithmetic Refinement:** Verifies constraints like `x > 5` at compile time.

---

## 8. Honest Assessment: Gaps & Limitations

To maintain academic integrity, we explicitly list what the system **does not** currently handle. These are the differences between a **Model Checker** (LOGOS) and a **Proof Assistant** (like Coq or Lean).

### 8.1 No Derivation Trees (`⊢`)
*   **Gap:** The system cannot generate a human-readable, step-by-step proof (e.g., "Line 3: Modus Ponens on 1, 2").
*   **Reality:** We provide **Satisfiability** (Sat/Unsat), not **Derivation**.

### 8.2 Blind to Modal Axioms
*   **Gap:** Z3 treats `Accessible_Alethic` as just another predicate. It does not know if this relation is Reflexive (Axiom T), Transitive (Axiom 4), or Euclidean (Axiom 5).
*   **Consequence:** LOGOS cannot automatically prove theorems that rely on specific modal logic systems (like S4 or S5) without manual axiom injection.

### 8.3 No User-Configurable Accessibility
*   **Gap:** Users cannot currently specify properties of the accessibility relations (e.g., "Let Deontic accessibility be Serial").
*   **Status:** Hardcoded to generic accessibility.

---

## 9. Summary

LOGOS is a **Linguistic Semantic Engine** powered by **SMT Model Checking**.

*   **It IS:** A tool for translating natural language into rigorous Kripke semantics and checking for consistency.
*   **It is NOT:** A tool for generating natural deduction proof trees or verifying complex modal theorems requiring specific axiom systems (yet).
