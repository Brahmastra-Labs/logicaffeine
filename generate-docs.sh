#!/bin/bash

# LOGICAFFEINE 1.0 - AAA Documentation Generator
# Generates comprehensive markdown documentation for the English-to-First-Order-Logic transpiler

OUTPUT_FILE="LOGOS_DOCUMENTATION.md"

echo "Generating comprehensive LOGICAFFEINE documentation..."

# ==============================================================================
# HEADER & TABLE OF CONTENTS
# ==============================================================================
cat > "$OUTPUT_FILE" << 'EOF'
# LOGICAFFEINE 1.0 - Complete Source Documentation

An English-to-First-Order-Logic transpiler with modal operators, temporal logic, and semantic analysis.

**Technology Stack:**
- Rust (core transpiler)
- Dioxus 0.6 (web UI with Router)
- Bumpalo (arena allocation)
- WASM (browser deployment)

---

## Acknowledgments & History

Logicaffeine stands on the shoulders of giants. This project draws deep inspiration from **LogiCola**, the legendary logic tutorial software created by **Harry J. Gensler** at John Carroll University. For decades, LogiCola helped students worldwide master symbolic logic through interactive exercises and immediate feedback.

The creator of Logicaffeine first encountered LogiCola as a college student, and it sparked a lasting passion for formal logic and natural language processing. The pedagogical brilliance of Gensler's approach—breaking complex logical concepts into digestible, interactive exercises—directly influenced Logicaffeine's curriculum design.

While early prototypes referenced LogiCola's exercise format (LogiCola 3.0), **Logicaffeine 1.0** is a complete reimagining: a modern English-to-First-Order-Logic transpiler built from the ground up in Rust, featuring Montague semantics, Neo-Davidsonian event structures, and parse forest ambiguity resolution. The programming language component is called **LOGOS**.

We honor LogiCola's legacy while charting a new course—extending beyond tutorial software into a full formal semantics engine capable of translating natural English into rigorous logical notation.

---

## Table of Contents

### Overview
1. [Architecture Overview](#architecture-overview)

### Grammar & Semantics
2. [Grammar Rules](#grammar-rules)
   - [Sentence Patterns](#sentence-patterns)
   - [Quantifiers & Scope](#quantifier-scope)
   - [Modal & Temporal Operators](#modal-operators)
   - [Linguistic Phenomena](#linguistic-phenomena)
   - [Output Examples](#output-examples)
3. [Glossary](#glossary)

### Test Coverage
4. [Integration Tests](#integration-tests)
    - [Phase 1: Garden Path](#phase-1-garden-path)
    - [Phase 2: Polarity Items](#phase-2-polarity-items)
    - [Phase 3: Tense & Aspect](#phase-3-tense--aspect)
    - [Phase 4: Movement & Reciprocals](#phase-4-movement--reciprocals)
    - [Phase 5: Wh-Movement](#phase-5-wh-movement)
    - [Phase 6: Complex Tense](#phase-6-complex-tense)
    - [Phase 7: Intensional Semantics](#phase-7-intensional-semantics)
    - [Phase 8: Degrees & Comparatives](#phase-8-degrees--comparatives)
    - [Phase 9: Noun/Verb Conversion](#phase-9-nounverb-conversion)
    - [Phase 10: Ellipsis & Sluicing](#phase-10-ellipsis--sluicing)
    - [Phase 11: Sorts & Metaphor](#phase-11-sorts--metaphor)
    - [Phase 12: Parse Forest](#phase-12-parse-forest)
    - [Phase 13: Multi-Word Expressions](#phase-13-mwe)
    - [Phase 14: Ontology & Bridging](#phase-14-ontology)
    - [Phase 15: Negation & Polarity](#phase-15-negation--polarity)
    - [Phase 16: Aspect Stack](#phase-16-aspect-stack)
    - [Phase 17: Comparatives & Superlatives](#phase-17-comparatives--superlatives)
    - [Phase 18: Plurality](#phase-18-plurality)
    - [Phase 19: Group Plurals](#phase-19-group-plurals)
    - [Phase 20: Axiom Layer](#phase-20-axiom-layer)
    - [Phase 21: Block Structure & Imperative](#phase-21-block-headers)
    - [Phase 22: Identity, Scope & Resolution](#phase-22-identity-scope)
    - [Phase 23: Type System & Statements](#phase-23-type-system)
    - [Phase 24: Code Generation](#phase-24-code-generation)
    - [Phase 25: Assertions & Smoke Tests](#phase-25-assertions)
    - [Phase 26: End-to-End Pipeline](#phase-26-end-to-end)
    - [Phase 27: Guards](#phase-27-guards)
    - [Phase 28: Precedence](#phase-28-precedence)
    - [Phase 29: Runtime Injection](#phase-29-runtime-injection)
    - [Phase 30: Collections & Iteration](#phase-30-collections--iteration)
    - [Phase 31: User-Defined Types](#phase-31-user-defined-types)
    - [Phase 32: Function Definitions & Inference](#phase-32-function-definitions--inference)
    - [Phase 33: Sum Types & Pattern Matching](#phase-33-sum-types--pattern-matching)
    - [Phase 34: User-Defined Generics](#phase-34-user-defined-generics)
    - [Phase 35: The Proof Bridge](#phase-35-the-proof-bridge)
5. [Statistics](#statistics)

### Source Code
6. [Lexicon Data](#lexicon-data)
7. [Lexer & Tokenization](#lexer--tokenization)
8. [Parser & AST](#parser--ast) (Dual-AST: logic.rs + stmt.rs)
9. [Transpilation](#transpilation)
10. [Semantic Analysis](#semantic-analysis)
11. [Type Analysis](#type-analysis) (analysis/ module)
12. [Code Generation](#code-generation) (codegen.rs, compile.rs, scope.rs)
13. [Public API](#public-api)
14. [Linguistic Data](#linguistic-data)
15. [Memory Management](#memory-management)
16. [Error Handling](#error-handling)
17. [Gamification](#gamification) (achievements, progress, SRS)
18. [Web Application](#web-application)
    - [Pages](#pages): Home, Workspace, Pricing, Learn, Lesson
    - [Components](#components): ChatDisplay, InputArea
    - [Router](#router): Client-side navigation
19. [Problem Generator](#problem-generator)
    - [Curriculum Structure](#curriculum-structure)
    - [Runtime Lexicon](#runtime-lexicon)
    - [Generator Engine](#generator-engine)
    - [Grader](#grader)
20. [Logos Core Runtime](#logos-core-runtime)

### Appendix
16. [Metadata](#metadata)

---

## Architecture Overview

LOGICAFFEINE implements a compiler pipeline for natural language to formal logic translation, with support for **structural ambiguity** via parse forests.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           LOGICAFFEINE 1.0 Pipeline                         │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   ┌─────────┐    ┌─────────┐    ┌─────────┐    ┌───────────┐    ┌───────┐  │
│   │  Input  │───▶│  Lexer  │───▶│ Parser  │───▶│ Transpile │───▶│Output │  │
│   │ English │    │         │    │         │    │           │    │ FOL   │  │
│   └─────────┘    └────┬────┘    └────┬────┘    └─────┬─────┘    └───────┘  │
│                       │              │               │                      │
│                       ▼              ▼               ▼                      │
│                  ┌─────────┐    ┌─────────┐    ┌───────────┐               │
│                  │ Tokens  │    │  Parse  │    │ Vec<AST>  │               │
│                  └─────────┘    │  Forest │    │ (multiple │               │
│                                 └─────────┘    │ readings) │               │
│                                      │         └───────────┘               │
│                                      ▼                                      │
│                              ┌───────────────┐                              │
│                              │    Lambda     │                              │
│                              │   Calculus    │                              │
│                              │  (Semantics)  │                              │
│                              └───────────────┘                              │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Data Flow:**
1. **Lexer** (`lexer.rs`, `token.rs`): Tokenizes English input using dictionary-based classification
2. **Parser** (`parser/`, `ast.rs`): Builds Arena-based AST via recursive descent; returns **parse forest** for ambiguous inputs
3. **Semantics** (`lambda.rs`, `context.rs`): Lambda calculus for compositional meaning
4. **Transpiler** (`transpile.rs`, `formatter.rs`): Generates Unicode or LaTeX logical notation

**Key Design Decisions:**
- **Arena allocation** (`bumpalo`) for zero-copy AST nodes with `Copy` semantics
- **Boxed AST variants** - Large variants boxed to reduce Expr size (112→32 bytes)
- **AstContext** - Unified arena access struct for ergonomic allocation
- **Visitor pattern** - walk_expr/walk_term for clean AST traversal
- **RAII checkpoints** - ParserCheckpoint for automatic backtracking cleanup
- **ParserGuard** - RAII guard with Deref/DerefMut for transparent parser access and automatic rollback
- **Fluent builders** - Type-safe expression construction with inline builders on AstContext
- **Semantic token sets** - Const arrays (WH_WORDS, MODALS) for declarative token matching via check_any()
- **Zero-alloc transpile** - write_to/write_logic methods avoid String allocation
- **Parse forest** output via `compile_ambiguous()` for structurally ambiguous sentences
- **Neo-Davidsonian event semantics** with thematic roles (Agent, Patient, Theme, etc.)
- **Control theory** (Chomsky) - raising verbs, control verbs, PRO binding
- **Generic quantifiers** (`Gen x`) for law-like generalizations ("Birds fly")
- **Mereology/Plurals** with Sigma operator (σ) and Distributive wrapper (*)
- **Presupposition & Focus** semantics (Strawson, Rooth)
- **Gapping/ellipsis** support via backtracking and verb recovery
- Symbol interning for efficient string comparisons
- Discourse context tracking for pronoun resolution
- Socratic error messages for educational feedback
- **Span tracking** - Source positions on tokens for rich error diagnostics with display_with_source()
- **Snapshot testing** - Golden master tests via assert_snapshot! macro (UPDATE_SNAPSHOTS=1 to regenerate)
- **Typo suggestions** - Zero-dependency Levenshtein distance for 'did you mean?' error messages
- **ANSI styling** - Compiler-style colored terminal output for errors
- **Reciprocals** - "each other" expands to bidirectional predicate conjunction
- **Polarity sensitivity** - Context-aware "any" interpretation (NPI vs free choice)
- **Garden path reanalysis** - Reduced relative clause detection with backtracking
- **Aspect chains** - Perfect/progressive/passive/modal stacking in verb groups
- **Voice operator** - Passive voice handling integrated with event semantics
- **Vendler classes (Aktionsart)** - Lexical aspect classification: State, Activity, Accomplishment, Achievement, Semelfactive
- **Zero-derivation** - Nouns dynamically coerced to verbs via morphological heuristics; consonant cluster recovery for silent-e lemmas
- **VP Ellipsis reconstruction** - EventTemplate stores verb + non-agent roles; try_parse_ellipsis() detects auxiliary + terminator pattern and rebuilds NeoEvent with new subject
- **Sluicing reconstruction** - Wh-word at sentence boundary triggers reconstruction from last_event_template; contraction expansion in lexer enables "don't know who" parsing
- **Verb-first classification** - Polysemous words (love, ring) classified as verbs first; parser accepts verbs in noun positions via consume_content_word()
- **Parse forest** - compile_forest() returns Vec<String> of all valid readings for ambiguous sentences
- **MAX_FOREST_READINGS** (12) - Limits parse forest size to prevent exponential blowup
- **noun_priority_mode** - Parser flag for lexical ambiguity forking; prefers noun interpretation for Ambiguous tokens
- **Sort system** - Ontological type hierarchy (Human⊂Animate⊂Physical) for semantic compatibility and metaphor detection
- **MWE Pipeline** - Post-tokenization trie-based collapsing of multi-word expressions (compound nouns, idioms, phrasal verbs)
- **Bridging Anaphora** - Part-whole inference for definite NPs without direct antecedent via ontology lookup
- **Copula Adjective Preference** - After copula, simple-aspect Verbs with Adjective alternatives prefer adjective reading
- **Content Word Classifiers** - Heuristic disambiguation via is_noun_like(), is_verb_like(), is_adjective_like()
- **NPI Licensing** - Negative Polarity Items (any, ever, anything) require licensing context; "no/nobody/nothing" are inherently negative quantifiers
- **Collective/Distributive ambiguity** - Mixed verbs (lift, carry) fork parse forest for plural subjects; collective verbs (gather, meet) force group reading; distributive verbs force individual reading
- **Distributive Expr** - Expr::Distributive wraps predicates for \`*\` operator transpilation
- **GroupQuantifier** - Expr::GroupQuantifier for cardinal indefinites with collective readings; outputs Group(g) ∧ Count(g, n) ∧ ∀x(Member(x, g) → R(x)) structure
- **Axiom Layer** - Post-parse AST transformation via apply_axioms(); expands predicates using meaning postulates from lexicon (analytic entailments, privatives, hypernyms, verb entailments)
- **Problem Generator** - Template-based exercise generation with {ProperName}, {Noun}, {Verb}, {Adjective} slots; runtime lexicon queries with constraint filtering; morphological modifiers (:Plural, :Past)
- **Semantic Grading** - Answer comparison via Unicode normalization, AST parsing, and structural equivalence; handles commutativity of ∧/∨; partial credit scoring
- **Curriculum Embedding** - Filesystem-based curriculum (assets/curriculum/) embedded at compile time via include_dir; JSON schemas for eras, modules, exercises
- **Catch-all 404 Route** - NotFound variant with /:..route pattern prevents router panics on invalid URLs

**Quantifier Kinds:**
| Kind | Symbol | Example | Meaning |
|------|--------|---------|---------|
| Universal | ∀ | "All birds fly" | Every individual |
| Existential | ∃ | "Some bird flies" | At least one |
| Generic | Gen | "Birds fly" | Characteristic/law-like |
| Negative | ¬∃ | "No birds swim" | None |
| Many | MANY | "Many dogs bark" | Significantly many |
| Most | MOST | "Most birds fly" | More than half |
| Few | FEW | "Few cats swim" | Small number |

**Scope Enumeration Complexity:**
| Scenario | Formula | Example |
|----------|---------|---------|
| Naive | n! | 3 quantifiers → 6 readings |
| With Islands | Π(k_i!) | 4 quantifiers (2+2) → 4 readings |
| + Intensionality | Π(k_i!) × 2^m | m opaque verbs add binary choice |

**Vendler Classes (Aktionsart):**
| Class | Features | Example Verbs | Meaning |
|-------|----------|---------------|---------|
| State | +static, +durative, -telic | know, love, exist | No change, extends in time |
| Activity | -static, +durative, -telic | run, swim, drive | Dynamic, no endpoint |
| Accomplishment | -static, +durative, +telic | build, draw, write | Dynamic with endpoint |
| Achievement | -static, -durative, +telic | win, find, die | Instantaneous change |
| Semelfactive | -static, -durative, -telic | knock, cough, blink | Single punctual event |

**Verb Plurality Classes:**
| Class | Example Verbs | Plural Subject Behavior |
|-------|---------------|------------------------|
| Collective | gather, meet, disperse | Group reading only |
| Distributive | sleep, run, die | Individual reading only |
| Mixed | lift, carry, surround | Ambiguous - forks readings |

**Word Classification Priority:**
| Word | In Verbs | In disambiguation_not_verbs | In Nouns | In Adjectives | Result |
|------|----------|----------------------------|----------|---------------|--------|
| love | ✓ | ✗ | ✓ | ✗ | Verb (parser handles noun positions) |
| ring | ✓ | ✓ | ✓ | ✗ | Noun (disambiguation + noun check) |
| bus  | ✓ | ✓ | ✓ | ✗ | Noun (disambiguation + noun check) |
| fake | ✓ | ✓ | ✗ | ✗ | Adjective (disambiguation, not noun) |
| open | ✓ | ✗ | ✗ | ✓ | Ambiguous{Verb, [Adj]} (copula prefers Adj) |

**Lexical Ambiguity (Phase 12):**
| Pattern | Example | Readings |
|---------|---------|----------|
| Noun/Verb | "I saw her duck" | duck=bird vs duck=action |
| Verb/Adjective | "The door is open" | open=Adj (copula preference) vs open=Verb |
| Possessive Pronoun | "her book" vs "saw her" | possessive determiner vs object pronoun |
| PP Attachment | "man with telescope" | VP attachment (instrument) vs NP attachment (modifier) |

**Sort Hierarchy (Phase 11):**
| Sort | Parent | Examples |
|------|--------|----------|
| Human | Animate | John, Mary, Juliet |
| Animate | Physical | dog, cat, bird |
| Celestial | - | Sun, Moon, stars |
| Abstract | - | Time, Justice, Love |
| Physical | - | Rock, Table, Book |
| Value | - | Money, Gold |

**Aspect Operators:**
| Operator | Symbol | Example | Meaning |
|----------|--------|---------|---------|
| Progressive | Prog | "is running" | Ongoing action |
| Perfect | Perf | "has eaten" | Completed with relevance |
| Habitual | HAB | "John runs" (present activity) | Characteristic behavior |
| Iterative | ITER | "kept knocking" | Repeated semelfactive |

**Plural Semantics (Link-style Mereology):**
| Feature | Syntax | Output | Meaning |
|---------|--------|--------|---------|
| Sigma operator | "The dogs" | σx.Dog(x) | Maximal sum of all dogs |
| Collective verb | "The dogs gathered" | P(G(σD)) | Group action |
| Distributive verb | "The dogs barked" | *P(B(σD)) | Each individual acted |
| Coordination | "John and Mary met" | P(M2(J ⊕ M)) | Sum of individuals |

**Event Semantics (Neo-Davidsonian):**
| Role | Example | Output |
|------|---------|--------|
| Agent | "John kicked the ball" | ∃e(Kick(e) ∧ Agent(e,j) ∧ Theme(e,b)) |
| Theme | "The ball was kicked" | ∃e(Kick(e) ∧ Theme(e,b)) |
| Recipient | "John gave Mary a book" | ∃e(Give(e) ∧ Agent(e,j) ∧ Recipient(e,m) ∧ Theme(e,b)) |
| Instrument | "with a hammer" | Instrument(e,h) |

**Ditransitive Verbs:**
| Verb | Example | Roles |
|------|---------|-------|
| give | "John gave Mary a book" | Agent, Recipient, Theme |
| send | "Mary sent John a letter" | Agent, Recipient, Theme |
| tell | "She told him a story" | Agent, Recipient, Theme |

**Causal Relations:**
| Type | Example | Output |
|------|---------|--------|
| Because | "John fell because he slipped" | Cause(Slip(j), Fall(j)) |

**Deixis (Demonstratives):**
| Type | Words | Predicate |
|------|-------|-----------|
| Proximal | this, these | Proximal(x) |
| Distal | that, those | Distal(x) |

**Gerunds:**
| Position | Example | Output |
|----------|---------|--------|
| Subject | "Running is healthy" | Healthy(Running) |
| Object | "John loves swimming" | Love(j, Swimming) |

**Mass Nouns:**
| Measure | Example | Output |
|---------|---------|--------|
| Much | "much water" | Measure(x, Much) ∧ Water(x) |
| Little | "little time" | Measure(x, Little) ∧ Time(x) |

**Control Theory (Chomsky):**
| Type | Example | Structure |
|------|---------|-----------|
| Subject Control | "John wants to leave" | Want(j, PRO_j leave) |
| Object Control | "John persuaded Mary to go" | Persuade(j, m, PRO_m go) |
| Raising | "John seems to be happy" | Seem(Happy(j)) |

**Adjective Types:**
| Type | Example | Output | Semantics |
|------|---------|--------|-----------|
| Intersective | "a red ball" | R(x) ∧ B(x) | Independent predicates |
| Subsective | "a small elephant" | S(x, ^E) ∧ E(x) | Relative to noun class |
| Non-Intersective | "a fake gun" | Fake(Gun) | Modifies concept |

**Measurement Semantics (Phase 8):**
| Dimension | Example | Output |
|-----------|---------|--------|
| Length | "5 meters long" | Value(5, meters, Length) |
| Temperature | "98.6 degrees" | Value(98.6, degrees, Temperature) |
| Cardinality | "aleph_0" | Value(aleph_0, ∅, Cardinality) |
| Comparative | "2 inches taller" | Taller(j, m, Value(2, inches)) |

**Compound Identifiers:**
| Pattern | Example | Output |
|---------|---------|--------|
| noun + label | "set A" | set_A |
| noun + proper | "King John" | King_John |
| noun + letter | "function F" | function_F |

**Zero-Derivation (Phase 9):**
| Pattern | Example | Output |
|---------|---------|--------|
| noun→verb (past) | "tabled the motion" | Table(committee, motion) |
| noun→verb (past) | "emailed him" | Email(she, him) |
| noun→verb (past) | "googled the answer" | Google(j, answer) |
| noun→verb (modal) | "should table" | Modal(Should, Table(x, motion)) |

**VP Ellipsis (Phase 10a):**
| Pattern | Example | Output |
|---------|---------|--------|
| does too | "John runs. Mary does too." | Run(j) ∧ Run(m) |
| modal too | "John can swim. Mary can too." | ◇Swim(j) ∧ ◇Swim(m) |
| does not | "John runs. Mary does not." | Run(j) ∧ ¬Run(m) |
| with object | "John eats an apple. Mary does too." | Eat(j,apple) ∧ Eat(m,apple) |

**Sluicing (Phase 10b):**
| Pattern | Example | Output |
|---------|---------|--------|
| who sluicing | "Someone left. I know who." | ∃x(Leave(x)) ∧ Know(I, ?y[Leave(y)]) |
| what sluicing | "John ate something. I know what." | ∃x(Eat(j,x)) ∧ Know(I, ?y[Eat(j,y)]) |
| negation | "Someone called. I don't know who." | ∃x(Call(x)) ∧ ¬Know(I, ?y[Call(y)]) |
| wonder | "Someone ran. I wonder who." | ∃x(Run(x)) ∧ Wonder(I, ?y[Run(y)]) |

---

EOF

# ==============================================================================
# HELPER FUNCTIONS
# ==============================================================================
add_file() {
    local file_path="$1"
    local title="$2"
    local description="$3"

    if [ -f "$file_path" ]; then
        echo "Adding: $file_path"

        local lang="rust"
        case "$file_path" in
            *.toml) lang="toml" ;;
            *.sh) lang="bash" ;;
            *.md) lang="markdown" ;;
            *.json) lang="json" ;;
        esac

        cat >> "$OUTPUT_FILE" << SECTION_END
### $title

**File:** \`$file_path\`

$description

\`\`\`$lang
SECTION_END
        cat "$file_path" >> "$OUTPUT_FILE"
        echo "" >> "$OUTPUT_FILE"
        cat >> "$OUTPUT_FILE" << 'SECTION_END'
```

---

SECTION_END
    else
        echo "Warning: File not found: $file_path"
    fi
}

# Test description helper - includes description and example without source code
add_test_description() {
    local file_path="$1"
    local title="$2"
    local description="$3"
    local example="$4"

    if [ -f "$file_path" ]; then
        echo "Adding test: $file_path"
        cat >> "$OUTPUT_FILE" << SECTION_END
#### $title

**File:** \`$file_path\`

$description

**Example:** $example

---

SECTION_END
    fi
}

# Line counting helper for statistics
count_lines_in() {
    local files="$@"
    local total=0
    for f in $files; do
        if [ -f "$f" ]; then
            total=$((total + $(wc -l < "$f")))
        fi
    done
    echo $total
}
# GRAMMAR RULES (inline with Parser section)
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'

## Grammar Rules

LOGICAFFEINE parses English sentences according to the following grammar patterns:

### Sentence Patterns

| Pattern | Example | Logic |
|---------|---------|-------|
| Universal | "All cats are mammals" | ∀x(Cat(x) → Mammal(x)) |
| Existential | "Some dogs bark" | ∃x(Dog(x) ∧ Bark(x)) |
| Generic | "Birds fly" | Gen x(Bird(x) → Fly(x)) |
| Singular | "Socrates is mortal" | Mortal(socrates) |
| Negative | "No fish fly" | ¬∃x(Fish(x) ∧ Fly(x)) |
| Conditional | "If it rains, the ground is wet" | Rain → Wet(ground) |
| Gapping | "John ate an apple, and Mary, a pear" | Ate(john, apple) ∧ Ate(mary, pear) |
| Plural Collective | "The dogs gathered" | P(G(σD)) |
| Plural Distributive | "The dogs barked" | *P(B(σD)) |
| Coordination | "John and Mary met" | P(M2(J ⊕ M)) |
| Subject Control | "John wants to leave" | Want(j, Leave(PRO_j)) |
| Object Control | "John persuaded Mary to go" | Persuade(j, m, Go(PRO_m)) |
| Raising | "John seems to be happy" | Seem(Happy(j)) |
| Focus | "Only John ran" | Only(j, Ran(j)) |
| Presupposition | "John stopped smoking" | Smoke(j)_presup ∧ ¬Smoke(j) |
| Counterfactual | "If John had run, he would have won" | □→(Run(j), Win(j)) |
| Comparative | "John is taller than Mary" | Taller(j, m) |
| Ditransitive | "John gave Mary a book" | ∃e(Give(e) ∧ Agent(e,j) ∧ Recipient(e,m) ∧ Theme(e,b)) |
| Causal | "John fell because he slipped" | Cause(Slip(j), Fall(j)) |
| Gerund Subject | "Running is healthy" | Healthy(Running) |
| Gerund Object | "John loves swimming" | Love(j, Swimming) |
| Deixis Proximal | "This cat meows" | ∃x(Proximal(x) ∧ Cat(x) ∧ Meow(x)) |
| Deixis Distal | "That dog barks" | ∃x(Distal(x) ∧ Dog(x) ∧ Bark(x)) |
| Mass Noun | "Much water flows" | ∃x(Measure(x, Much) ∧ Water(x) ∧ Flow(x)) |
| Reciprocal | "They love each other" | Love(x,y) ∧ Love(y,x) |
| NPI Any | "No one has any books" | ¬∃x∃y(Person(x) ∧ Book(y) ∧ Has(x,y)) |
| Free Choice Any | "Any book works" | ∀x(Book(x) → Works(x)) |
| Garden Path | "The horse raced past the barn fell" | ∃x(Horse(x) ∧ RacedPast(x,barn) ∧ Fell(x)) |
| Perfect Aspect | "John has eaten" | Perf(Eat(j)) |
| Progressive | "John is eating" | Prog(Eat(j)) |
| Passive Voice | "The ball was kicked" | ∃e(Kick(e) ∧ Theme(e,ball)) |
| Contact Clause | "The cat the dog chased ran" | ∃x(Cat(x) ∧ ∃y(Dog(y) ∧ Chase(y,x)) ∧ Run(x)) |
| Stacked Relatives | "Every book that John read that Mary wrote" | ∀x((Book(x) ∧ Read(j,x) ∧ Wrote(m,x)) → ...) |

### Quantifier Kinds

| Kind | Trigger | Symbol | Semantics |
|------|---------|--------|-----------|
| Universal | "all", "every", "each" | ∀ | True for every individual |
| Existential | "some", "a", "an" | ∃ | True for at least one |
| Generic | Bare plural ("birds") | Gen | Law-like/characteristic |
| Negative | "no", "none" | ¬∃ | True for none |

### Quantifier Scope

- Nested quantifiers resolve left-to-right by default
- "Every student read some book" → ∀x(Student(x) → ∃y(Book(y) ∧ Read(x,y)))
- Scope can be disambiguated via context

### Structural Ambiguity

Sentences with multiple valid parses return all readings via `compile_ambiguous()`:

| Sentence | Reading 1 | Reading 2 |
|----------|-----------|-----------|
| "I saw the man with the telescope" | See(i, man, with:telescope) | See(i, man) ∧ Has(man, telescope) |

**PP-Attachment:** Prepositional phrases can attach to VP (instrument) or NP (modifier).

### Modal Operators

| Operator | Symbol | Meaning |
|----------|--------|---------|
| Necessity | □ | "must", "necessarily" |
| Possibility | ◇ | "can", "possibly", "might" |
| Obligation | O | "ought", "should" |
| Permission | P | "may" (deontic) |

### Temporal Operators

| Operator | Symbol | Meaning |
|----------|--------|---------|
| Future | F | "will" |
| Past | P | "did", past tense |
| Always | G | "always" |
| Sometimes | F | "sometimes" |

### Linguistic Phenomena

LOGICAFFEINE supports the following linguistic constructs:

### Quantification
- Universal: "all", "every", "each", "any"
- Existential: "some", "a", "an", "there exists"
- Generic: bare plurals ("birds", "dogs") - law-like generalizations
- Negative: "no", "none", "not any"

### Plurals & Mereology (Link-style)
- Sigma operator (σ): maximal sum - "the dogs" → σx.Dog(x)
- Collective verbs: "gather", "meet", "assemble", "disperse" - no distributive wrapper
- Distributive verbs: most verbs - wrapped with * operator
- Group formation (⊕): "John and Mary" → J ⊕ M

### Event Semantics (Neo-Davidsonian)
- Event variables: ∃e(Kick(e) ∧ Agent(e,j) ∧ Theme(e,b))
- Thematic roles: Agent, Patient, Theme, Goal, Source, Recipient, Instrument, Location, Time, Manner
- Adverbial modification: Manner(e, quickly)

### Control Theory (Chomsky)
- Subject control: "want", "try", "promise" - PRO bound to subject
- Object control: "persuade", "force", "convince" - PRO bound to object
- Raising: "seem", "appear" - no PRO, subject raises

### Presupposition (Strawson)
- Factive triggers: "know", "regret", "realize"
- Aspectual triggers: "stop", "start", "continue" (require gerund complement)
  - "John stopped smoking." → presupposition: John was smoking
  - "John stopped." → no presupposition (simple past tense verb)
- Definite descriptions: presuppose existence

### Focus (Rooth)
- Focus particles: "only", "even", "just"
- Alternative semantics: focus introduces alternatives

### Connectives
- Conjunction: "and", "but", "yet"
- Disjunction: "or", "either...or"
- Implication: "if...then", "implies", "only if"
- Biconditional: "if and only if", "iff"

### Modality
- Alethic: "necessarily", "possibly", "must", "can"
- Deontic: "ought", "should", "may", "permitted"
- Epistemic: "knows", "believes"

### Anaphora
- Pronouns: "he", "she", "it", "they"
- Reflexives: "himself", "herself", "itself"
- Demonstratives: "this", "that"

### Relative Clauses
- Restrictive: "the man who runs"
- Non-restrictive: "John, who is tall"
- Stacked relatives: "the book that John read that Mary wrote" - multiple relative clauses on single head noun

### Presupposition Triggers
- Definite descriptions: "the king of France"
- Factive verbs: "knows that", "regrets that"
- Change of state: "stopped", "began" (gerund complement required)
  - Triggered: "stopped smoking" → presupposes was smoking
  - Not triggered: "stopped" alone → simple verb, no presupposition

### Ellipsis & Gapping
- Gapping: "John ate an apple, and Mary, a pear" (verb elided in second conjunct)
- VP Ellipsis: "John ran and Mary did too"
- Sluicing: "Someone left, but I don't know who"

### Structural Ambiguity
- PP-attachment: "I saw the man with the telescope" (VP vs NP attachment)
- Quantifier scope: "Every boy loves some girl" (wide vs narrow scope)
- Coordination: "old men and women" (old men + women vs old men + old women)

### Deixis (Demonstratives)
- Proximal: "this", "these" → Proximal(x) predicate
- Distal: "that", "those" → Distal(x) predicate
- Spatial reference relative to speaker position

### Ditransitive Verbs
- Double object construction: "give X Y", "send X Y", "tell X Y"
- Recipient thematic role for indirect object
- Three-argument event: Agent, Recipient, Theme

### Gerunds
- Gerund as subject: "Running is fun" → predicate applied to nominalized verb
- Gerund as object: "I love swimming" → verb as theme argument
- -ing form functioning as noun phrase

### Causal Connectives
- "because" introduces causal relation: Cause(antecedent, consequent)
- Antecedent is the cause, consequent is the effect
- Order: "X because Y" → Cause(Y, X)

### Mass Nouns
- "much" + noun → Measure(x, Much) ∧ Noun(x)
- "little" + noun → Measure(x, Little) ∧ Noun(x)
- Quantity expressions for uncountable nouns

### Reciprocals
- Pattern: "each other" with plural subject
- Expansion: P(x,y) ∧ P(y,x) for all pairs in the group
- Example: "John and Mary love each other" → Love(j,m) ∧ Love(m,j)

### Polarity Items
- NPI "any": Licensed by negation, questions, conditionals → existential
- Free choice "any": Universal interpretation in positive contexts
- Context tracking via negation depth in parser
- Example: "No one has any books" (NPI) vs "Any book will do" (free choice)

### Garden Path Sentences
- Reduced relative clauses: "The horse raced past the barn fell"
- Initially parsed as main verb, triggers reanalysis
- Backtracking to reduced relative interpretation
- Parser uses try_reduced_relative_interpretation()

### Vendler Classes (Aktionsart)
- State: +static, +durative, -telic (know, love) - no progressive allowed
- Activity: -static, +durative, -telic (run, swim) - present tense → Habitual
- Accomplishment: -static, +durative, +telic (build, write) - present tense → Habitual
- Achievement: -static, -durative, +telic (win, die) - present tense → Habitual
- Semelfactive: -static, -durative, -telic (knock, blink) - progressive → Iterative

### Aspect System
- Progressive: -ing form (is/was running) → Prog(φ)
- Perfect: have/has/had + past participle → Perf(φ)
- Habitual: present tense non-stative → HAB(φ)
- Iterative: semelfactive + progressive → ITER(φ)
- Passive: been + past participle → Voice(Passive, φ)
- Chains: Modal + Perfect + Passive + Progressive stacking
- Example: "would have been being eaten" → chains all four

### Contact Clauses
- Reduced relatives without overt relativizer: "The cat the dog chased ran"
- Pattern: NP + NP + Verb triggers contact clause interpretation
- Equivalent to: "The cat [that] the dog chased ran"
- Parser uses is_contact_clause_pattern() lookahead

### Intensionality (De Re / De Dicto)
- De Re: Object exists in actual world, quantifier scopes wide
- De Dicto: Object described intensionally, quantifier scopes narrow
- Opaque verbs: "seek", "want", "believe", "need", "fear"
- Montague up-arrow (^): Marks intension of predicate
- Example: "John seeks a unicorn"
  - De Re: ∃x(Unicorn(x) ∧ Seek(j, x)) - specific unicorn exists
  - De Dicto: Seek(j, ^Unicorn) - seeking unicorn-concept

### Scope Islands
- Island boundaries: conjunctions (if/and/or), relative clauses
- Quantifiers cannot scope out of their island
- Reduces exponential scope ambiguity to manageable product of factorials
- Example: "Every man runs AND some dog barks" → 1! × 1! = 1 reading (no cross-island scoping)

### Adjective Semantics
- Intersective: Property independent of noun - "red ball" → R(x) ∧ B(x)
- Subsective: Property relative to noun class - "small elephant" → S(x, ^E) ∧ E(x)
- Non-Intersective: Modifies concept - "fake gun" → Fake(Gun)
- Gradable: Allows comparison - "taller", "tallest"
- Montague up-arrow (^): Marks intension of noun for subsective context

### Generalized Quantifiers
- Beyond ∀/∃: MANY, MOST, FEW with cardinality semantics
- MANY x(P(x) ∧ Q(x)) - significantly many P's are Q's
- MOST x(P(x) → Q(x)) - more than half of P's are Q's
- FEW x(P(x) ∧ Q(x)) - small number of P's are Q's

### Zero-Derivation (Noun→Verb Conversion)
- English allows nouns to be used as verbs without morphological marking
- Pattern detection: Past tense "-ed" suffix on non-lexicon words
- Morphological recovery: Silent-e restoration via consonant cluster heuristics
- Example: "table" (noun) → "tabled" (verb) → "Table(x, y)"
- Handles: tabled, emailed, googled, skyped, friended, texted, etc.

### VP Ellipsis (Anaphoric Reconstruction)
- Elided VP reconstructed from discourse antecedent
- Pattern: Subject + Auxiliary + (not)? + Terminator (too/also/.)
- Template stores: verb + non-agent thematic roles + modifiers
- Reconstruction: New subject as Agent, preserve Theme/Goal/etc.
- Modal preservation: "can too" → same modal on reconstructed VP
- Negation: "does not" → negated reconstruction
- Examples: "John runs. Mary does too." → Run(j) ∧ Run(m)

### Sluicing (Wh-Ellipsis)
- Elided wh-clause reconstructed from discourse antecedent
- Pattern: Embedding verb + wh-word + terminator (period/comma/EOF)
- Template stores: verb + thematic roles from antecedent clause
- Reconstruction: wh-variable fills Agent (who) or Theme (what) slot
- Negation support: "I don't know who" via contraction expansion
- Embedding verbs: know, wonder (Opaque feature)
- Examples: "Someone left. I know who." → ∃x(Leave(x)) ∧ Know(I, ?y[Leave(y)])

### Degree Semantics (Phase 8)
- Comparative with measure: "2 inches taller" → Taller(j, m, 2 inches)
- Absolute measurement: "5 meters long" → Long(rope, 5 meters)
- Symbolic numbers: aleph_0, omega for infinite cardinals
- Dimension tracking: Length, Time, Weight, Temperature, Cardinality
- NumberKind types: Real(f64), Integer(i64), Symbolic(Symbol)
- Term::Value stores numeric value with optional unit and dimension

### Topicalization (Object Fronting)
- Pattern: "NP, Subject Verb" → fronted NP is object (Theme)
- Example: "The apple, John ate." → Eat(j, apple)
- Adjective preservation: "The red apple, John ate." keeps Red(x) ∧ Apple(x)
- Pronoun subject handling: "The book, he read."
- Implementation: parser/mod.rs lines 401-473, uses wrap_with_definiteness_full()

### Long-Distance Dependencies (Wh-Movement)
- Filler-Gap binding across clause boundaries
- Example: "Who did John say Mary loves?" → λx.Say(j, Loves(m, x))
- Parser field: filler_gap: Option<Symbol> in Parser struct
- Persists through recursive clause parsing for embedded extractions
- Pied-piping: "To whom did John give the book?" fronts P+wh together

### Sentential Complements (Embedded Clauses)
- Verbs like "say", "believe", "think" take clausal arguments
- Represented as Term::Proposition wrapping the embedded Expr
- Example: "John said Mary runs" → Say(j, [Run(m)])
- Bracket notation [expr] distinguishes from conjunction
- Supports wh-extraction across clause boundaries
- Communication verbs: say, tell, report, announce
- Propositional attitude verbs: believe, think, know, doubt

### Reichenbach Temporal Semantics
- Three-point temporal model: Event (E), Reference (R), Speech (S)
- Past: R < S (reference before speech)
- Future: S < R (speech before reference)
- Perfect: E < R (event before reference)
- Past Perfect: E < R < S ("had run")
- Future Perfect: E < R, S < R ("will have run")
- Present Perfect: E < R, R = S ("has run")
- Output uses Precedes(x, y) predicates for explicit temporal ordering

### Multi-Word Expressions (Phase 13)
- Compound nouns: "fire engine" → FireEngine (merged single token)
- Idioms: "kicked the bucket" → Die (semantic replacement)
- Phrasal verbs: "gave up" → Surrender
- Trie-based pattern matching for efficient MWE detection
- Tense inheritance: "kicked the bucket" inherits past tense from "kicked"
- Post-tokenization pipeline: apply_mwe_pipeline() collapses multi-token sequences

### Bridging Anaphora (Phase 14)
- Part-whole inference for definite NPs without direct antecedent
- Example: "I bought a car. The engine smoked." → PartOf(engine, car)
- Ontology lookup: find_bridging_wholes() returns possible whole objects
- Ambiguous bridging handled via parse forest forking
- Sort compatibility checking for semantic validation

### Metaphor Detection (Phase 14)
- Sort violations trigger Metaphor wrapper
- Example: "The rock was happy" → Metaphor(Happy(rock))
- Predicate sort requirements checked via ontology module
- Compatible sorts pass without Metaphor wrapping
- Example: "John was happy" → Happy(j) (no metaphor - Human compatible with mental predicates)

### Negation & Polarity Items (Phase 15)
- Free choice "any": "Any cat hunts" → ∀x(Cat(x) → Hunt(x))
- NPI "any" with negation: "did not see any X" → ¬∃x(X(x) ∧ See(...,x))
- Negative quantifiers: nobody, nothing, no one → ∀x(R(x) → ¬P(x))
- Scope: "Not all birds fly" → ¬∀ (negation scopes over universal)
- Temporal NPIs: "never" → inherent negation, "ever" → requires licensor
- NPI licensing by "no": "No dog saw anything" licenses existential in object

### Output Examples

#### Unicode Output

**Input:** "All humans are mortal"
**Output:** `∀x(Human(x) → Mortal(x))`

**Input:** "Birds fly" (Generic)
**Output:** `Gen x(Bird(x) → Fly(x))`

**Input:** "Socrates is human and Socrates is mortal"
**Output:** `Human(socrates) ∧ Mortal(socrates)`

**Input:** "John ate an apple, and Mary, a pear" (Gapping)
**Output:** `Ate(john, apple) ∧ Ate(mary, pear)`

**Input:** "Necessarily, if something is a bachelor then it is unmarried"
**Output:** `□∀x(Bachelor(x) → Unmarried(x))`

### Plural Output (Mereology)

**Input:** "The dogs gathered" (Collective)
**Output:** `P(G(σD))`

**Input:** "The dogs barked" (Distributive)
**Output:** `*P(B(σD))`

**Input:** "John and Mary met" (Coordination)
**Output:** `P(M2(J ⊕ M))`

### Event Semantics Output

**Input:** "John kicked the ball"
**Output:** `∃e(Kick(e) ∧ Agent(e,j) ∧ Theme(e,b))`

### Control Output

**Input:** "John wants to leave"
**Output:** `Want(j, Leave(PRO_j))`

**Input:** "John seems to be happy"
**Output:** `Seem(Happy(j))`

### Focus & Presupposition Output

**Input:** "Only John ran"
**Output:** `Only(j, Ran(j))`

**Input:** "John stopped smoking"
**Output:** `Stop(j, Smoke(j))` with presupposition: `Smoke(j)`

### Ambiguous Output (compile_ambiguous)

**Input:** "I saw the man with the telescope"
**Output (Reading 1 - VP attachment):** `See(i, man, with:telescope)`
**Output (Reading 2 - NP attachment):** `See(i, man) ∧ Has(man, telescope)`

### LaTeX Output

**Input:** "All humans are mortal"
**Output:** `\forall x(Human(x) \rightarrow Mortal(x))`

**Input:** "Birds fly" (Generic)
**Output:** `\text{Gen } x(Bird(x) \rightarrow Fly(x))`

**Input:** "Some cat loves every dog"
**Output:** `\exists x(Cat(x) \land \forall y(Dog(y) \rightarrow Loves(x,y)))`

### Ditransitive Output

**Input:** "John gave Mary a book"
**Output:** `∃e(Give(e) ∧ Agent(e, j) ∧ Recipient(e, m) ∧ Theme(e, b))`

**Input:** "She sent him a letter"
**Output:** `∃e(Send(e) ∧ Agent(e, s) ∧ Recipient(e, h) ∧ Theme(e, l))`

### Causal Output

**Input:** "John fell because he slipped"
**Output:** `Cause(Slip(j), Fall(j))`

**Input:** "The plant died because it lacked water"
**Output:** `Cause(Lack(p, w), Die(p))`

### Gerund Output

**Input:** "Running is healthy"
**Output:** `Healthy(Running)`

**Input:** "John loves swimming"
**Output:** `Love(j, Swimming)`

### Deixis Output

**Input:** "This dog barks"
**Output:** `∃x(Proximal(x) ∧ Dog(x) ∧ Bark(x))`

**Input:** "Those cats meow"
**Output:** `∃x(Distal(x) ∧ Cat(x) ∧ Meow(x))`

### Mass Noun Output

**Input:** "Much water flows"
**Output:** `∃x(Measure(x, Much) ∧ Water(x) ∧ Flow(x))`

**Input:** "Little time remains"
**Output:** `∃x(Measure(x, Little) ∧ Time(x) ∧ Remain(x))`

### Reciprocal Output

**Input:** "John and Mary love each other"
**Output:** `Love(j, m) ∧ Love(m, j)`

**Input:** "They saw each other"
**Output:** `See(x, y) ∧ See(y, x)`

### Polarity Output

**Input:** "No one has any books" (NPI)
**Output:** `¬∃x(Person(x) ∧ ∃y(Book(y) ∧ Has(x, y)))`

**Input:** "Any book works" (Free Choice)
**Output:** `∀x(Book(x) → Works(x))`

### Garden Path Output

**Input:** "The horse raced past the barn fell"
**Output:** `∃x(Horse(x) ∧ RacedPast(x, barn) ∧ Fell(x))`

### Aspect Output

**Input:** "John is running" (Progressive)
**Output:** `Prog(∃e(Run(e) ∧ Agent(e, j)))`

**Input:** "John has eaten" (Perfect)
**Output:** `Perf(∃e(Eat(e) ∧ Agent(e, j)))`

**Input:** "The ball was kicked" (Passive)
**Output:** `∃e(Kick(e) ∧ Theme(e, ball))`

**Input:** "John was running" (Past Progressive)
**Output:** `Past(Prog(∃e(Run(e) ∧ Agent(e, j))))`

### Vendler/Aktionsart Output

**Input:** "John runs" (Activity, Present)
**Output:** `HAB(∃e(Run(e) ∧ Agent(e, j)))`

**Input:** "John knows Mary" (State, Present)
**Output:** `∃e(Know(e) ∧ Agent(e, j) ∧ Theme(e, m))` (no Habitual wrapper)

**Input:** "John is knocking" (Semelfactive, Progressive)
**Output:** `ITER(∃e(Knock(e) ∧ Agent(e, j)))`

**Input:** "John built a house" (Accomplishment, Past)
**Output:** `Past(∃e(Build(e) ∧ Agent(e, j) ∧ Theme(e, h)))`

**Input:** "John won" (Achievement, Past)
**Output:** `Past(∃e(Win(e) ∧ Agent(e, j)))`

### Intensional Readings (De Re / De Dicto)

**Input:** "John seeks a unicorn"
**Output (De Re):** `∃x(Unicorn(x) ∧ ∃e(Seek(e) ∧ Agent(e, j) ∧ Theme(e, x)))`
**Output (De Dicto):** `∃e(Seek(e) ∧ Agent(e, j) ∧ Theme(e, ^Unicorn))`

**Input:** "Mary needs a doctor"
**Output (De Re):** `∃x(Doctor(x) ∧ Need(m, x))` - specific doctor
**Output (De Dicto):** `Need(m, ^Doctor)` - any doctor

**Input:** "John believes a spy exists"
**Output (De Re):** `∃x(Spy(x) ∧ Believe(j, Exists(x)))` - specific spy
**Output (De Dicto):** `Believe(j, ∃x(Spy(x)))` - belief in existence

### Adjective Output

**Input:** "A small elephant ran." (Subsective)
**Output:** `∃x(S(x, ^E) ∧ E(x) ∧ ∃e(Run(e) ∧ Agent(e, x)))`

**Input:** "A red ball rolled." (Intersective)
**Output:** `∃x(R(x) ∧ B(x) ∧ ∃e(Roll(e) ∧ Agent(e, x)))`

**Input:** "A large mouse ran." (Subsective)
**Output:** `∃x(L(x, ^M) ∧ M(x) ∧ ∃e(Run(e) ∧ Agent(e, x)))`

### Generalized Quantifier Output

**Input:** "Many dogs bark."
**Output:** `MANY x(D(x) ∧ B(x))`

**Input:** "Most birds fly."
**Output:** `MOST x(B(x) ∧ F(x))`

**Input:** "Few cats swim."
**Output:** `FEW x(C(x) ∧ S(x))`

### Measurement Output (Phase 8)

**Input:** "John is 2 inches taller than Mary."
**Output:** `Taller(j, m, 2 inches)`

**Input:** "The rope is 5 meters long."
**Output:** `Long(rope, 5 meters)`

**Input:** "Set A has cardinality aleph_0."
**Output:** `Cardinality(A, aleph_0)`

**Input:** "The temperature is 98.6 degrees."
**Output:** `Temperature(t, 98.6 degrees)`

### Zero-Derivation Output (Phase 9)

**Input:** "The committee tabled the discussion."
**Output:** `∃e(Table(e) ∧ Agent(e, committee) ∧ Theme(e, discussion))`

**Input:** "She emailed him."
**Output:** `∃e(Email(e) ∧ Agent(e, she) ∧ Theme(e, him))`

**Input:** "John googled the answer."
**Output:** `∃e(Google(e) ∧ Agent(e, j) ∧ Theme(e, answer))`

### VP Ellipsis Output (Phase 10a)

**Input:** "John runs. Mary does too."
**Output:** `Run(j) ∧ Run(m)`

**Input:** "John can swim. Mary can too."
**Output:** `◇Swim(j) ∧ ◇Swim(m)`

**Input:** "John runs. Mary does not."
**Output:** `Run(j) ∧ ¬Run(m)`

**Input:** "John eats an apple. Mary does too."
**Output:** `∃e(Eat(e) ∧ Agent(e,j) ∧ Theme(e,apple)) ∧ ∃e(Eat(e) ∧ Agent(e,m) ∧ Theme(e,apple))`

### Sluicing Output (Phase 10b)

**Input:** "Someone left. I know who."
**Output:** `∃x(Leave(x)) ∧ Know(I, Question(y, Leave(y)))`

**Input:** "John ate something. I know what."
**Output:** `∃x(Eat(j,x)) ∧ Know(I, Question(y, Eat(j,y)))`

**Input:** "Someone called. I don't know who."
**Output:** `∃x(Call(x)) ∧ ¬Know(I, Question(y, Call(y)))`

**Input:** "Someone ran. I wonder who."
**Output:** `∃x(Run(x)) ∧ Wonder(I, Question(y, Run(y)))`

### Topicalization Output

**Input:** "The apple, John ate."
**Output:** `∃x(((Apple(x) ∧ ∀y((Apple(y) → y = x))) ∧ Eat(J, x)))`

**Input:** "The red apple, John ate."
**Output:** `∃x(((Red(x) ∧ Apple(x) ∧ ∀y((...) → y = x))) ∧ Eat(J, x)))`

**Input:** "A book, Mary read."
**Output:** `∃x((Book(x) ∧ Read(M, x)))` - indefinite topic

### Long-Distance Wh-Movement Output

**Input:** "Who did John say Mary loves?"
**Output:** `λx.Say(J, Love(M, x))` - gap filled in embedded clause

**Input:** "To whom did John give the book?"
**Output:** `λx.Give(J, book, x)` - pied-piped preposition

### Embedded Clause (Sentential Complement) Output

**Input:** "John said Mary runs."
**Output:** `Say(J, [Run(M)])` - embedded clause as argument with bracket notation

**Input:** "Who did John say Mary loves?"
**Output:** `λx.Past(Say(J, [Love(M, x)]))` - gap filled in embedded Proposition

**Input:** "John believes Mary won."
**Output:** `Believe(J, [Past(Win(M))])` - propositional attitude with clause argument

### Reichenbach Temporal Output

**Input:** "John had run." (Past Perfect)
**Output:** `Precedes(e, r) ∧ Precedes(r, S) ∧ Run(e, j)` - E < R < S

**Input:** "John will have run." (Future Perfect)
**Output:** `Precedes(S, r) ∧ Precedes(e, r) ∧ Run(e, j)` - S < R, E < R

**Input:** "John has run." (Present Perfect)
**Output:** `Precedes(e, r) ∧ Run(e, j)` - E < R (R = S implicit)

---

## Glossary

### First-Order Logic Terms

| Term | Definition |
|------|------------|
| **Predicate** | A property or relation: P(x), Loves(x,y) |
| **Quantifier** | Binds variables: ∀ (universal), ∃ (existential), Gen (generic) |
| **Generic Quantifier** | Law-like generalization over a kind: "Birds fly" → Gen x(Bird(x) → Fly(x)) |
| **Variable** | A placeholder: x, y, z |
| **Constant** | A named individual: socrates, fido |
| **Connective** | Logical operators: ∧ (and), ∨ (or), → (implies), ↔ (iff), ¬ (not) |
| **Formula** | A well-formed logical expression |
| **Scope** | The extent of a quantifier's binding |
| **Free Variable** | A variable not bound by any quantifier |
| **Bound Variable** | A variable within a quantifier's scope |

### Modal Logic Terms

| Term | Definition |
|------|------------|
| **Necessity (□)** | True in all possible worlds |
| **Possibility (◇)** | True in at least one possible world |
| **Alethic** | Concerning truth and necessity |
| **Deontic** | Concerning obligation and permission |
| **Epistemic** | Concerning knowledge and belief |

### Linguistic Terms

| Term | Definition |
|------|------------|
| **Noun Phrase** | A noun with modifiers: "the tall man" |
| **Verb Phrase** | A verb with complements: "loves Mary" |
| **Relative Clause** | A clause modifying a noun: "who runs" |
| **Anaphora** | Reference to a previous expression |
| **Definiteness** | Whether a noun is specific: "the" vs "a" |
| **Aspect** | Temporal structure of events |
| **Thematic Role** | Semantic role: agent, patient, theme |
| **Gapping** | Ellipsis of a verb in coordination: "John ate an apple, and Mary, a pear" |
| **PP-Attachment** | Where a prepositional phrase attaches: VP (instrument) or NP (modifier) |
| **Bare Plural** | Plural noun without determiner: "birds" (triggers generic reading) |
| **Collective Verb** | Verb requiring group action: "gather", "meet", "disperse" |
| **Distributive Verb** | Verb applying to each individual: "bark", "run", "sleep" |
| **Mereology** | Theory of parts and wholes; used for plural semantics |
| **Thematic Role** | Semantic role in event: Agent, Patient, Theme, Goal, etc. |
| **Control Verb** | Verb where embedded subject is controlled: "want", "try" |
| **Raising Verb** | Verb where subject raises from embedded clause: "seem" |
| **PRO** | Silent pronoun in infinitival clauses bound by controller |
| **Presupposition** | Background assumption triggered by certain expressions |
| **Focus Particle** | Word highlighting alternatives: "only", "even", "just" |
| **Counterfactual** | Conditional contrary to fact: "if...had...would" |
| **Deixis** | Contextual reference: "this/that" (proximal/distal), "here/there", "now/then" |
| **Ditransitive Verb** | Verb taking two objects: "give", "send", "tell" (Agent, Recipient, Theme) |
| **Gerund** | Verb form functioning as noun: "Running is fun", "I love swimming" |
| **Causal Connective** | Word linking cause to effect: "because" → Cause(antecedent, consequent) |
| **Mass Noun** | Uncountable noun: "water", "rice", "information" (quantified by "much"/"little") |
| **Reciprocal** | Bidirectional relation with "each other": P(x,y) ∧ P(y,x) |
| **NPI (Negative Polarity Item)** | Words like "any" requiring negative context for existential reading |
| **Free Choice Any** | Universal "any" in positive contexts: ∀x |
| **Garden Path** | Sentence requiring structural reanalysis due to initial misparse |
| **Reduced Relative** | Relative clause without "who/that": "the man seen" = "the man who was seen" |
| **Perfect Aspect** | Completed action with current relevance: "has eaten" → Perf(φ) |
| **Progressive Aspect** | Ongoing action: "is eating" → Prog(φ) |
| **Habitual Aspect** | Present tense activity/accomplishment/achievement interpretation: "runs" → HAB(Run(x)) |
| **Iterative Aspect** | Progressive semelfactive producing repeated event: "is knocking" → ITER(Knock) |
| **Aspect Chain** | Stacked aspect operators: modal + perfect + passive + progressive |
| **Contact Clause** | Relative clause without overt "who/that": "the man I saw" = "the man that I saw" |
| **Vendler Class** | Lexical aspect category: State, Activity, Accomplishment, Achievement, Semelfactive |
| **Stative** | Verb class feature (+static): no change over time (know, love, exist) |
| **Durative** | Verb class feature (+durative): extends over time (run, build) vs punctual (win, knock) |
| **Telic** | Verb class feature (+telic): has natural endpoint (build, win) vs atelic (run, know) |
| **Semelfactive** | Punctual, atelic verb class: single events (knock, blink, cough) |
| **Aktionsart** | German term for lexical aspect; verb-inherent temporal properties (synonym: Vendler class) |
| **Subsective Adjective** | Adjective whose meaning depends on noun class: "small" relative to elephants vs mice |
| **Intersective Adjective** | Adjective forming independent predicate: "red" applies regardless of noun |
| **Non-Intersective Adjective** | Adjective modifying the noun concept itself: "fake gun" |
| **Generalized Quantifier** | Quantifiers beyond ∀/∃: MANY, MOST, FEW with cardinality semantics |
| **Degree Phrase** | Measure expression modifying comparison: "2 inches" in "2 inches taller" |
| **Absolute Measurement** | Direct dimension attribution: "5 meters long" |
| **Symbolic Number** | Mathematical constant: aleph_0, omega for infinite cardinals |
| **Compound Identifier** | Noun followed by proper name or single letter label: "set A" → set_A |
| **Zero-Derivation** | Conversion of a word from one category to another without morphological change: "table" (noun) → "table" (verb) |
| **VP Ellipsis** | Omission of a verb phrase that is recoverable from context: "John runs. Mary does too." = Mary runs |
| **Sluicing** | Ellipsis of a wh-clause recoverable from context: "Someone left. I know who." = I know who left |

### Implementation Terms

| Term | Definition |
|------|------------|
| **Token** | A classified unit from the lexer |
| **Lexeme** | The actual text of a token |
| **AST** | Abstract Syntax Tree (arena-allocated with Copy semantics) |
| **Parse Forest** | Multiple AST trees for ambiguous sentences; returned by compile_ambiguous() |
| **Recursive Descent** | Top-down parsing strategy with backtracking for ellipsis |
| **Precedence** | Operator binding strength |
| **Interning** | Storing strings once, referencing by ID |
| **Arena Allocation** | Batch memory allocation via bumpalo; enables Copy AST nodes |
| **Beta Reduction** | Lambda calculus substitution |
| **Backtracking** | Parser technique for handling gapping by rewinding and retrying |
| **Sigma (σ)** | Term constructor for maximal sum of a predicate: σx.Dog(x) |
| **Distributive (*)** | Expression wrapper for distributive readings over plurals |
| **Group (⊕)** | Term constructor for sum of individuals: J ⊕ M |
| **NeoEvent** | Expression with event variable and thematic role assignments |
| **Control** | Expression for control/raising verb structures |
| **ThematicRole** | Enum: Agent, Patient, Theme, Goal, Source, Recipient, Instrument, Location, Time, Manner |
| **Recipient** | Thematic role for indirect object in ditransitive verbs |
| **Causal** | Expression variant representing cause-effect relationships: Cause(antecedent, consequent) |
| **MeasureKind** | Enum for quantity expressions: Much, Little (used with mass nouns) |
| **AstContext** | Unified struct holding all arena allocators for AST construction |
| **ParserCheckpoint** | RAII struct for parser backtracking with automatic restore |
| **ParserGuard** | RAII struct with Deref for transparent parser access; auto-restores on drop unless commit() called |
| **Visitor** | Trait for traversing AST nodes without manual recursion |
| **Fluent builders** | Inline methods on AstContext (binary, unary, quantifier, temporal, aspectual, modal) for ergonomic AST construction |
| **Semantic token sets** | Const arrays (WH_WORDS, MODALS) grouping related tokens for check_any() matching |
| **Zero-alloc transpile** | Output methods using Write trait to avoid String allocation |
| **Span** | Byte range (start, end) for source location tracking on tokens |
| **display_with_source()** | Renders ParseError with line numbers and underline markers pointing to error location |
| **assert_snapshot!** | Macro for golden master testing; compares output against stored snapshots in tests/snapshots/ |
| **Levenshtein distance** | Edit distance algorithm for finding similar words; used for 'did you mean?' suggestions |
| **find_similar()** | Finds closest vocabulary match within threshold for typo correction |
| **Style** | ANSI color wrapper with red(), blue(), cyan(), green(), bold_red() methods |
| **VoiceOperator** | Enum for voice handling in AST: Passive variant |
| **VerbClass** | Enum for Vendler categories: State, Activity, Accomplishment, Achievement, Semelfactive |
| **VerbEntry** | Struct with lemma, time, aspect, and class fields for verb dictionary entries |
| **is_stative()** | VerbClass method returning true for State class |
| **is_durative()** | VerbClass method returning true for State, Activity, Accomplishment |
| **is_telic()** | VerbClass method returning true for Accomplishment, Achievement |
| **is_negative_context()** | Parser method tracking negation depth for NPI licensing |
| **is_followed_by_gerund()** | Parser helper checking if presupposition trigger is followed by gerund; prevents false presupposition on bare "stopped" |
| **parse_aspect_chain()** | Parser method handling complex verb group stacking |
| **parse_aspect_chain_with_term()** | Parser method for aspect chains with variable subjects (used in relative clause + modal combinations) |
| **Stacked Relatives** | Multiple relative clauses modifying same head noun: "the book that X read that Y wrote" |
| **try_reduced_relative_interpretation()** | Parser method for garden path reanalysis |
| **is_contact_clause_pattern()** | Parser lookahead for NP+NP+Verb contact clause detection |
| **Island** | Scope boundary preventing quantifier extraction: if/and/or clauses |
| **island_id** | u32 field on Quantifier identifying its scope island |
| **enumerate_scopings()** | Function returning ScopeIterator over all quantifier scope readings |
| **ScopeIterator** | Lazy iterator implementing ExactSizeIterator for scope readings |
| **group_by_island()** | Groups quantifiers by island_id to constrain permutations |
| **De Re** | "Of the thing" - object exists, quantifier scopes wide |
| **De Dicto** | "Of the word" - intensional reading, quantifier scopes narrow |
| **Opaque Verb** | Verb creating intensional context: seek, want, believe, need, fear |
| **is_opaque_verb()** | Function checking if a verb creates an opaque context |
| **Intension (^)** | Montague up-arrow marking concept/property vs individual |
| **Term::Intension** | Term variant for intensional predicates: ^Unicorn |
| **Expr::Intensional** | Expression wrapper for opaque verb contexts |
| **substitute_respecting_opacity()** | Substitution that blocks inside intensional contexts |
| **enumerate_intensional_readings()** | Generates de re and de dicto readings for opaque verb sentences |
| **IntensionalContext** | Struct tracking opaque verb, quantifier variable, and restrictor |
| **compile_all_scopes()** | Public API returning all scope + intensionality readings |
| **Topicalization** | Object fronting with comma intonation break: "NP, Subject Verb" pattern |
| **filler_gap** | Parser field (Option<Symbol>) tracking wh-filler for long-distance dependencies |
| **Long-Distance Dependency** | Extraction across clause boundaries: "Who did John say Mary loves?" |
| **Pied-Piping** | P+wh fronting: "To whom" instead of "Who...to" |
| **wrap_with_definiteness_full()** | NounPhrase wrapper preserving adjectives during topicalization |
| **Phase 4 Movement Tests** | tests/phase4_movement.rs: topicalization, adjective preservation, pronoun subjects |
| **Term::Proposition** | Term variant wrapping embedded Expr for sentential complements |
| **Sentential Complement** | Clause serving as argument: "John said [Mary runs]" |
| **Bracket Notation [expr]** | Transpilation format for embedded clauses to distinguish from conjunction |
| **Phase 5 Wh-Movement Tests** | tests/phase5_wh_movement.rs: long-distance extraction, embedded clauses, double embedding |
| **Reichenbach Semantics** | Three-point temporal model with E (event), R (reference), S (speech) |
| **Event Point (E)** | When the event occurs in Reichenbach model |
| **Reference Point (R)** | Temporal vantage point for viewing event |
| **Speech Point (S)** | Time of utterance |
| **Precedes(x, y)** | Temporal ordering predicate: x before y |
| **Phase 6 Complex Tense Tests** | tests/phase6_complex_tense.rs: Reichenbach E/R/S temporal constraints |
| **is_subsective()** | Generated function checking if adjective is subsective (relative to class) |
| **QuantifierKind::Many** | AST variant for generalized "many" quantifier |
| **QuantifierKind::Most** | AST variant for generalized "most" quantifier |
| **QuantifierKind::Few** | AST variant for generalized "few" quantifier |
| **Term::Intension** | Term variant for Montague up-arrow notation (^Noun) in subsective contexts |
| **Dimension** | Enum for measurement categories: Length, Time, Weight, Temperature, Cardinality |
| **NumberKind** | Enum for numeric types: Real(f64), Integer(i64), Symbolic(Symbol) |
| **Term::Value** | Term variant storing numeric value with optional unit Symbol and Dimension |
| **Comparative.difference** | Optional field for measure phrase in comparative expressions |
| **TokenType::Number** | Token variant storing numeric literal as interned Symbol |
| **parse_measure()** | Parser method for measure phrase expressions |
| **Phase 8 Degree Tests** | tests/phase8_degrees.rs: numeric measurement and degree semantics |
| **check_proper_name_or_label()** | Parser helper detecting proper names or single uppercase letter labels for compound identifier parsing |
| **Passive Agent Extraction** | Pattern matching "by X" after passive "been" to identify the semantic agent in passive constructions |
| **Consonant Cluster Heuristic** | Morphological rule: vowel + consonant + l/r at word end suggests silent-e lemma recovery (tabl → table) |
| **Phase 9 Zero-Derivation Tests** | tests/phase9_conversion.rs: noun→verb conversion with silent-e recovery |
| **EventTemplate** | Struct storing verb + non-agent thematic roles + modifiers for VP ellipsis reconstruction |
| **Phase 10a VP Ellipsis Tests** | tests/phase10_ellipsis.rs: VP ellipsis with does too, modal too, negation, and objects |
| **Contraction Expansion** | Lexer splits negative contractions: don't→do+not, won't→will+not, can't→cannot |
| **Phase 10b Sluicing Tests** | tests/phase10b_sluicing.rs: sluicing with who/what, negation, embedding verbs |
| **Verb-First Priority** | Classification order: verbs checked before nouns in lexer. Parser safety net via consume_content_word() accepts Verb tokens in noun positions. |
| **disambiguation_not_verbs** | Lexicon list of words that should NOT be classified as verbs despite having verb forms (ring, bus). Returns Noun if also in nouns list. |
| **Polysemy Resolution** | Handling words with multiple parts of speech. Verb-first + parser safety net enables "I love you" and "Love is real" from same token type. |
| **compile_forest()** | Phase 12 API returning Vec<String> of all valid parse readings for ambiguous sentences. |
| **MAX_FOREST_READINGS** | Constant (12) limiting parse forest size to prevent exponential blowup. |
| **noun_priority_mode** | Parser flag that prefers noun interpretation for Ambiguous tokens; used for lexical ambiguity forking. |
| **TokenType::Ambiguous** | Token variant with primary interpretation and alternatives Vec for polysemous words (duck, bear, love). |
| **Sort** | Phase 11 ontological type category: Human, Animate, Celestial, Abstract, Physical, Value. |
| **lookup_sort()** | Returns Sort for proper names; used for semantic type checking. |
| **is_compatible_with()** | Sort method checking type subsumption (Human⊂Animate⊂Physical). |
| **Lexical Ambiguity** | Words with multiple parts of speech requiring parse forest (e.g., "duck" as Noun or Verb). |
| **Structural Ambiguity** | Syntactic attachment ambiguity (PP attachment, coordination scope) handled via pp_attachment_mode. |
| **MweTrie** | Trie data structure for multi-word expression pattern storage and efficient longest-match lookup. |
| **apply_mwe_pipeline()** | Post-tokenization function that collapses multi-token MWE sequences into single tokens. |
| **build_mwe_trie()** | Creates default MWE vocabulary trie with compound nouns, idioms, and phrasal verbs. |
| **find_bridging_wholes()** | Ontology function returning possible whole objects for a given part noun (e.g., "engine" → ["car", "plane"]). |
| **check_sort_compatibility()** | Validates predicate-subject sort match; returns true if compatible or no requirement exists. |
| **PartOf** | Term relation representing part-whole relationship in bridging anaphora (e.g., PartOf(engine, car)). |
| **Bridging Anaphora** | Pragmatic inference linking a definite NP to an antecedent's part (e.g., "a car... the engine" → engine is part of car). |
| **Copula Adjective Preference** | Parser heuristic: after copula (is/was), simple-aspect Verbs with Adjective alternative prefer Adjective reading. |
| **is_adjective_like()** | Lexer heuristic checking if word could be an adjective for ambiguity detection. |
| **is_noun_like()** | Lexer heuristic checking if word could be a noun for ambiguity detection. |
| **is_verb_like()** | Lexer heuristic checking if word could be a verb for disambiguation. |
| **NPI (Negative Polarity Item)** | Words like "any", "ever", "anything" that require negative context for existential interpretation. |
| **Free Choice Any** | "Any" in affirmative contexts producing universal quantification: "Any cat hunts" → ∀x. |
| **Negative Quantifier** | Inherently negative quantifiers (nobody, nothing, no one) that produce ∀x(R(x) → ¬P(x)). |
| **NPI Licensing** | Process by which negative context triggers existential interpretation of NPIs. |

---

EOF

# TESTS (descriptions only, no source code)
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Integration Tests

Comprehensive test suite validating parsing and transpilation across 15 linguistic phases.

**Location:** `tests/`

EOF

# Phase 1
add_test_description "tests/phase1_garden_path.rs" \
    "Phase 1: Garden Path" \
    "Garden path sentences requiring structural reanalysis. Parser detects reduced relatives via backtracking when initial parse leaves unparsed tokens." \
    "\"The horse raced past the barn fell.\" → ∃x(Horse(x) ∧ RacedPast(x, barn) ∧ Fell(x))"

# Phase 2
add_test_description "tests/phase2_polarity.rs" \
    "Phase 2: Polarity Items" \
    "Negative Polarity Items (NPIs). 'any' is existential in negative/conditional contexts, universal in positive contexts. Parser tracks negative_depth for NPI licensing." \
    "\"Not any dogs run.\" → ¬∃x(D(x) ∧ R(x)) vs \"Any dog runs.\" → ∀x(D(x) → R(x))"

# Phase 3
add_test_description "tests/phase3_time.rs" \
    "Phase 3: Temporal Logic" \
    "Reichenbach temporal semantics with Event (E), Reference (R), and Speech (S) points. Tests simple tense, perfect aspect, and temporal anchoring." \
    "\"John had run.\" (past perfect) → Precedes(e, r) ∧ Precedes(r, S)"

add_test_description "tests/phase3_aspect.rs" \
    "Phase 3: Aspect & Aktionsart" \
    "Vendler aspectual classes (State/Activity/Accomplishment/Achievement/Semelfactive) and grammatical aspect interaction. Habitual for present-tense activities." \
    "\"John is knocking.\" (semelfactive+prog) → ITER(Knock(j))"

# Phase 4
add_test_description "tests/phase4_movement.rs" \
    "Phase 4: Topicalization" \
    "Filler-gap dependencies and object fronting. Tests NP-fronting with adjective preservation and pronoun subjects." \
    "\"The apple, John ate.\" → Eat(J, apple) with fronted Theme"

add_test_description "tests/phase4_reciprocals.rs" \
    "Phase 4: Reciprocals" \
    "Reciprocal 'each other' expands to bidirectional predicate conjunction for plural subjects." \
    "\"John and Mary love each other.\" → Love(j,m) ∧ Love(m,j)"

# Phase 5
add_test_description "tests/phase5_wh_movement.rs" \
    "Phase 5: Wh-Movement" \
    "Long-distance wh-dependencies across embedded clauses. Filler-gap binding through subordinate clauses with Term::Proposition wrapping." \
    "\"Who did John say Mary loves?\" → λx.Say(J, [Love(M, x)])"

# Phase 6
add_test_description "tests/phase6_complex_tense.rs" \
    "Phase 6: Complex Tense" \
    "Reichenbach temporal constraints with explicit E/R/S point relations. Verifies Precedes() predicate output." \
    "Past perfect: E < R < S; Future perfect: S < R, E < R"

# Phase 7
add_test_description "tests/phase7_semantics.rs" \
    "Phase 7: Intensional Semantics" \
    "Subsective adjectives (S(x, ^E) format) and generalized quantifiers (MANY, MOST, FEW with cardinality semantics)." \
    "\"A small elephant ran.\" → ∃x(S(x, ^E) ∧ E(x) ∧ Run(x))"

add_test_description "tests/intensionality_tests.rs" \
    "Phase 7: De Re/De Dicto" \
    "Intensional ambiguity with opaque verbs. Tests both readings for sentences with seek, want, believe, need, fear." \
    "\"John seeks a unicorn.\" → De Re: ∃x(U(x) ∧ Seek(j,x)) vs De Dicto: Seek(j, ^Unicorn)"

# Phase 8
add_test_description "tests/phase8_degrees.rs" \
    "Phase 8: Degrees & Comparatives" \
    "Numeric measurements and degree semantics. Tests comparatives with measure phrases, absolute measurements, and symbolic cardinality." \
    "\"John is 2 inches taller than Mary.\" → Taller(j, m, 2 inches)"

# Phase 9
add_test_description "tests/phase9_conversion.rs" \
    "Phase 9: Noun/Verb Conversion" \
    "Zero-derivation (noun→verb): tabled, emailed, googled. Morphological heuristics for silent-e lemma recovery." \
    "\"The committee tabled the motion.\" → Table(committee, motion)"

# Phase 10
add_test_description "tests/phase10_ellipsis.rs" \
    "Phase 10a: VP Ellipsis" \
    "VP ellipsis reconstruction via EventTemplate. Handles 'does too', modal ellipsis, negative ellipsis, and ellipsis with objects." \
    "\"John runs. Mary does too.\" → Run(j) ∧ Run(m)"

add_test_description "tests/phase10b_sluicing.rs" \
    "Phase 10b: Sluicing" \
    "Sluicing reconstruction: wh-words at sentence boundary after embedding verbs. Handles contractions (don't know who)." \
    "\"Someone left. I know who.\" → ∃x(Leave(x)) ∧ Know(I, ?y[Leave(y)])"

# Phase 11
add_test_description "tests/phase11_sorts.rs" \
    "Phase 11: Ontological Sorts" \
    "Sort system with type hierarchy (Human⊂Animate⊂Physical). Sort compatibility checking for semantic validation." \
    "Sort::Human.is_compatible_with(Sort::Animate) → true"

add_test_description "tests/phase11_metaphor.rs" \
    "Phase 11: Metaphor Detection" \
    "Metaphor detection via sort violations. Distinguishes literal copula from metaphorical assertions." \
    "\"The king is bald.\" → literal; \"Juliet is the sun.\" → sort violation → metaphor"

# Phase 12
add_test_description "tests/phase12_ambiguity.rs" \
    "Phase 12: Parse Forest" \
    "Lexical and structural ambiguity handling. compile_forest() returns Vec of all valid readings for ambiguous sentences." \
    "\"I saw her duck.\" → 2 readings (duck=Noun vs duck=Verb)"

# Phase 13
add_test_description "tests/phase13_mwe.rs" \
    "Phase 13: Multi-Word Expressions" \
    "MWE processing: compound nouns (fire engine → FireEngine), idioms (kicked the bucket → Die), phrasal verbs (gave up → Surrender). Trie-based pipeline collapses multi-token sequences into single semantic units." \
    "\"John kicked the bucket.\" → Die(j)"

# Phase 14
add_test_description "tests/phase14_ontology.rs" \
    "Phase 14: Ontology & Bridging" \
    "Bridging anaphora for part-whole inference (PartOf relation). Metaphor detection via sort violations. Sort compatibility checking for predicates." \
    "\"I bought a car. The engine smoked.\" → PartOf(engine, car)"

# Phase 15
add_test_description "tests/phase15_negation.rs" \
    "Phase 15: Negation & Polarity" \
    "NPI processing: free choice 'any' (universal), NPI 'any' with negation (existential), negative quantifiers (nobody/nothing/no one), temporal NPIs (never/ever), scope interactions. Licensing determines existential vs universal interpretation." \
    "\"Any cat hunts.\" → ∀x(Cat(x) → Hunt(x)); \"John did not see any cat.\" → ¬∃x(Cat(x) ∧ See(j,x))"

# Phase 16
add_test_description "tests/phase16_aspect.rs" \
    "Phase 16: Aspect Stack" \
    "Complex aspect operator combinations: Perfect+Progressive, Perfect+Passive, Modal+Perfect+Progressive. Tests proper operator nesting without conflation (e.g., Perfect+Progressive should NOT imply Passive)." \
    "\"John has been eating apples.\" → Perf(Prog(Eat(j, apples)))"

# Phase 17
add_test_description "tests/phase17_degrees.rs" \
    "Phase 17: Comparatives & Superlatives" \
    "Extended degree semantics: comparatives with measure phrases, clausal comparative ellipsis, and superlatives with domain restriction. Superlatives expand to universal quantification over the comparison class." \
    "\"John climbed the highest mountain.\" → ∀x((Mountain(x) ∧ x ≠ m) → Higher(m, x))"

# Phase 18
add_test_description "tests/phase18_plurality.rs" \
    "Phase 18: Plurality" \
    "Collective vs distributive verb semantics. Mixed verbs fork readings for plural subjects (lifted → collective OR distributive). Collective verbs (gathered) force group reading. Distributive verbs (slept) force individual reading." \
    "\"The boys lifted the piano.\" → Collective: Lift(σB, piano) OR Distributive: *Lift(σB, piano)"

# Phase 19
add_test_description "tests/phase19_group_plurals.rs" \
    "Phase 19: Group Plurals" \
    "Group existential quantification for cardinal indefinites with collective readings. Cardinal + mixed verb forks into distributive (∃=n) and collective (Group/Count/Member) readings. Collective verbs force group reading." \
    "\"Two boys lifted a rock.\" → Collective: ∃g(Group(g) ∧ Count(g, 2) ∧ ∀x(Member(x, g) → B(x)) ∧ Lift(g, rock))"

# Phase 20
add_test_description "tests/phase20_axioms.rs" \
    "Phase 20: Axiom Layer" \
    "Semantic axiom expansion for meaning postulates. Bachelor→Unmarried∧Male∧Adult, privative adjectives (fake→¬N∧Resembles(^N)), verb entailments (murder→kill), and hypernym chains (dog→animal→mammal). Pipeline position: Parser→Axioms→Pragmatics." \
    "\"John is a bachelor.\" → B(J) ∧ Unmarried(J) ∧ Male(J) ∧ Adult(J)"

# Phase 21: Block Structure & Imperative Syntax
add_test_description "tests/phase21_block_headers.rs" \
    "Phase 21: Block Headers" \
    "Parsing ## Main and other block headers that trigger imperative mode. Block headers mark the transition from declarative logic to executable code." \
    "\"## Main\" triggers imperative parsing mode"

add_test_description "tests/phase21_imperative_verbs.rs" \
    "Phase 21: Imperative Verbs" \
    "Let/Set/Return statement parsing in imperative blocks. Let binds values, Set mutates, Return exits functions." \
    "\"Let x be 5.\" → let x = 5;"

add_test_description "tests/phase21_ownership.rs" \
    "Phase 21: Ownership" \
    "Rust-style ownership semantics via natural language verbs. Give performs moves, Show performs immutable borrows. Tracks owned/moved/borrowed states." \
    "\"Give x to f.\" → f(x) // x is moved"

# Phase 22: Identity & Scope
add_test_description "tests/phase22_equals.rs" \
    "Phase 22: Equality" \
    "Identity predicates and equality relations. Handles 'is equal to', 'is identical to', and numeric equality." \
    "\"x is equal to y\" → x = y"

add_test_description "tests/phase22_index.rs" \
    "Phase 22: Indexing" \
    "Array and collection indexing operations. Supports numeric indices and slice syntax." \
    "\"the third element of xs\" → xs[2]"

add_test_description "tests/phase22_is_rejection.rs" \
    "Phase 22: Is-Rejection" \
    "Filtering non-predicate uses of 'is' copula in imperative context. Distinguishes identity from predication." \
    "\"x is large\" vs \"x is 5\""

add_test_description "tests/phase22_resolution.rs" \
    "Phase 22: Resolution" \
    "Anaphora and reference resolution in imperative blocks. Resolves pronouns and definite descriptions to bound variables." \
    "\"Let x be 5. Return it.\" → it resolves to x"

add_test_description "tests/phase22_scope.rs" \
    "Phase 22: Scope" \
    "Variable scope and quantifier interactions in imperative code. Handles block scoping and shadowing." \
    "Block-level variable scoping"

# Phase 23: Type System & Statements
add_test_description "tests/phase23_blocks.rs" \
    "Phase 23: Blocks" \
    "Indentation-based block structure parsing. Python-style significant whitespace with Colon/Indent/Dedent tokens." \
    "Indent → block body → Dedent"

add_test_description "tests/phase23_parsing.rs" \
    "Phase 23: Parsing" \
    "Parser internals and mode switching between declarative and imperative modes. Tests ParserMode enum." \
    "Declarative mode ↔ Imperative mode"

add_test_description "tests/phase23_stmt.rs" \
    "Phase 23: Statements" \
    "Stmt enum variants: Let, Set, Call, If, While, Return, Assert, Give, Show. The imperative AST types." \
    "Stmt::Let { name, value }"

add_test_description "tests/phase23_tokens.rs" \
    "Phase 23: Tokens" \
    "Token type verification for imperative constructs. Tests Give, Show, Let, Set, Return, Assert token recognition." \
    "TokenType::Give, TokenType::Show"

add_test_description "tests/phase23_types.rs" \
    "Phase 23: Types" \
    "TypeRegistry and DiscoveryPass for two-pass compilation. First pass discovers type definitions, second pass resolves references." \
    "## Definition blocks → TypeRegistry"

# Phase 24: Code Generation
add_test_description "tests/phase24_codegen.rs" \
    "Phase 24: Code Generation" \
    "Rust code emission for literals and expressions. Converts imperative AST to valid Rust source code." \
    "Stmt → fn main() { ... }"

add_test_description "tests/phase24_wired_types.rs" \
    "Phase 24: Pipeline Wiring" \
    "Two-pass compilation pipeline integration. DiscoveryPass runs before parser to build TypeRegistry. Parser uses registry for type vs predicate disambiguation." \
    "Stack of Integers → Generic type when Stack is defined"

add_test_description "tests/phase25_type_expr.rs" \
    "Phase 25: Type Expressions" \
    "Type annotations for Let statements. Supports primitives (Int→i64, Nat→u64, Text→String), generics (List of Int→Vec<i64>), multi-param generics (Result of Int and Text), nested generics, and mutable bindings." \
    "Let x: Int be 5. → let x: i64 = 5;"

# Phase 25: Assertions (separate from smoke tests)
add_test_description "tests/phase25_assertions.rs" \
    "Phase 25: Assertions" \
    "Logic kernel assertions via Assert statements. Bridges imperative code to declarative verification using debug_assert! macros." \
    "\"Assert that x is positive.\" → debug_assert!(x > 0)"

add_test_description "tests/phase25_smoke_tests.rs" \
    "Phase 25: Smoke Tests" \
    "Aspirational tests for advanced linguistic phenomena. Covers scopal adverbs (almost/barely wrapping events), negation scope ambiguity, donkey anaphora, intensional identity, performatives, distanced phrasal verbs, and double focus operators. Some tests expected to fail until features implemented." \
    "\"John almost killed Mary.\" → Almost(∃e(Kill(e) ∧ Agent(e, J) ∧ Theme(e, M)))"

# Phase 26-28: Advanced Pipeline
add_test_description "tests/phase26_e2e.rs" \
    "Phase 26: End-to-End" \
    "Full pipeline tests: English → AST → Rust code. Tests compile_to_rust output for complete programs." \
    "English source → executable Rust"

add_test_description "tests/phase27_guards.rs" \
    "Phase 27: Guards" \
    "Guard clauses and conditional patterns. Handles 'if' conditions and pattern guards in function definitions." \
    "\"If x is negative, return 0.\" → guard clause"

add_test_description "tests/phase28_precedence.rs" \
    "Phase 28: Precedence" \
    "Operator precedence and associativity. Ensures correct parsing of complex expressions with mixed operators." \
    "a + b * c → a + (b * c)"

add_test_description "tests/phase29_runtime.rs" \
    "Phase 29: Runtime Injection" \
    "Embeds logos_core/ runtime into compiled programs. Type aliases (Nat, Int, Real, Text, Bool, Unit) and IO functions (show, read_line) per Spec §10.5 and §10.6.1." \
    "use logos_core::prelude::*; // Auto-injected"

add_test_description "tests/phase30_iteration.rs" \
    "Phase 30: Collections & Iteration" \
    "Seq<T> generic type, list literals [1, 2, 3], repeat loops (for x in list:), range syntax (from N to M), and Showable trait. Mode-dependent 'in' keyword handling." \
    "Repeat for x in [1, 2, 3]: → for x in vec![1, 2, 3]"

add_test_description "tests/phase31_structs.rs" \
    "Phase 31: User-Defined Types" \
    "Struct definitions with encapsulation. Syntax: 'A TypeName has: a [public] field, which is Type.' Constructor generation (new Type), field access (var's field), field mutations (Set var's field to value), and visibility modifiers (pub/private fields)." \
    "A Point has: a public x, which is Int."

add_test_description "tests/phase32_functions.rs" \
    "Phase 32: Function Definitions & Inference" \
    "User-defined functions with ## To [verb] syntax. Call expression syntax f(x, y) for use in expressions, return type inference from body, and dual call syntax (Call f with x. for statements, f(x) for expressions)." \
    "## To add (a: Int) and (b: Int): → fn add(a: i64, b: i64) -> i64"

add_test_description "tests/phase33_enums.rs" \
    "Phase 33: Sum Types & Pattern Matching" \
    "Algebraic data types with 'A Type is either:' syntax. Variant constructors with optional payloads (A Circle with radius value.), pattern matching via 'Inspect expr:' with match arms, and field bindings in patterns (When Circle (radius: r):)." \
    "A Shape is either: A Circle with a radius, which is Int."

add_test_description "tests/phase34_generics.rs" \
    "Phase 34: User-Defined Generics" \
    "Generic type parameters with 'of [T]' syntax. Single-param (A Box of [T] has:), multi-param (A Pair of [A] and [B] has:), generic enums (A Maybe of [T] is either:), and turbofish instantiation (new Box of Int → Box::<i64>::default())." \
    "A Box of [T] has: a value, which is T."

add_test_description "tests/phase35_proofs.rs" \
    "Phase 35: The Proof Bridge" \
    "Proof assertions with 'Trust that P because \"reason\".' syntax. Generates debug_assert! with justification comment. Includes variable 'a' disambiguation, number literals in propositions, irregular comparatives (less/more/better/worse), and because-string lookahead guards." \
    "Trust that n is greater than 0 because \"precondition\"."

add_test_description "tests/phase35_respectively.rs" \
    "Phase 35: Respectively Coordination" \
    "Pairwise coordination with 'respectively' adverb. Matches coordinated subjects with coordinated objects pairwise (John and Mary saw Tom and Jane respectively → See(J,T) ∧ See(M,J)). Includes RespectivelyLengthMismatch error for mismatched counts, dual code paths for pronoun and noun phrase subjects." \
    "John and Mary saw Tom and Jane respectively."

# Other tests
add_test_description "tests/aktionsart_tests.rs" \
    "Aktionsart/Vendler Classes" \
    "Tests for Vendler's lexical aspect classes and their interaction with aspectual operators." \
    "State (know), Activity (run), Accomplishment (build), Achievement (win), Semelfactive (knock)"

add_test_description "tests/complex_combinations.rs" \
    "Complex Operator Chains" \
    "Tests for complex modal + aspect + tense chains with proper operator nesting." \
    "Perfect + Passive + Progressive stacking"

add_test_description "tests/torture_tests.rs" \
    "Parser Stress Tests" \
    "Edge case stress tests: deeply nested structures, unusual word orders, boundary conditions." \
    "Deeply nested relative clauses and coordinations"

add_test_description "tests/integration_tests.rs" \
    "Core Integration Tests" \
    "Comprehensive tests covering quantifiers, modals, temporal logic, relative clauses, and basic parsing." \
    "Universal, existential, and generic quantification patterns"


# STATISTICS
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Statistics

EOF

echo "### By Compiler Stage" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"

# Lexer
LEXER_LINES=0
for f in src/token.rs src/lexer.rs; do
    if [ -f "$f" ]; then
        LEXER_LINES=$((LEXER_LINES + $(wc -l < "$f")))
    fi
done
echo "Lexer (token.rs, lexer.rs):           $LEXER_LINES lines" >> "$OUTPUT_FILE"

# Parser & AST
PARSER_LINES=0
for f in src/ast/*.rs; do
    if [ -f "$f" ]; then
        PARSER_LINES=$((PARSER_LINES + $(wc -l < "$f")))
    fi
done
for f in src/parser/*.rs; do
    if [ -f "$f" ]; then
        PARSER_LINES=$((PARSER_LINES + $(wc -l < "$f")))
    fi
done
echo "Parser (ast/, parser/):               $PARSER_LINES lines" >> "$OUTPUT_FILE"

# Transpilation
TRANSPILE_LINES=0
for f in src/transpile.rs src/formatter.rs src/registry.rs; do
    if [ -f "$f" ]; then
        TRANSPILE_LINES=$((TRANSPILE_LINES + $(wc -l < "$f")))
    fi
done
echo "Transpilation:                        $TRANSPILE_LINES lines" >> "$OUTPUT_FILE"

# Code Generation
CODEGEN_LINES=0
for f in src/codegen.rs src/compile.rs src/scope.rs; do
    if [ -f "$f" ]; then
        CODEGEN_LINES=$((CODEGEN_LINES + $(wc -l < "$f")))
    fi
done
echo "Code Generation:                      $CODEGEN_LINES lines" >> "$OUTPUT_FILE"

# Semantics
SEMANTICS_LINES=0
for f in src/lambda.rs src/context.rs src/view.rs; do
    if [ -f "$f" ]; then
        SEMANTICS_LINES=$((SEMANTICS_LINES + $(wc -l < "$f")))
    fi
done
echo "Semantics (lambda, context, view):    $SEMANTICS_LINES lines" >> "$OUTPUT_FILE"

# Type Analysis
ANALYSIS_LINES=0
for f in src/analysis/*.rs; do
    if [ -f "$f" ]; then
        ANALYSIS_LINES=$((ANALYSIS_LINES + $(wc -l < "$f")))
    fi
done
echo "Type Analysis (analysis/):            $ANALYSIS_LINES lines" >> "$OUTPUT_FILE"

# Support
SUPPORT_LINES=0
for f in src/lib.rs src/lexicon.rs src/intern.rs src/arena.rs src/error.rs src/debug.rs src/test_utils.rs; do
    if [ -f "$f" ]; then
        SUPPORT_LINES=$((SUPPORT_LINES + $(wc -l < "$f")))
    fi
done
echo "Support Infrastructure:               $SUPPORT_LINES lines" >> "$OUTPUT_FILE"

# UI
UI_LINES=0
if [ -d "src/ui" ]; then
    UI_LINES=$(find src/ui -name "*.rs" -exec cat {} \; 2>/dev/null | wc -l)
fi
echo "Desktop UI:                           $UI_LINES lines" >> "$OUTPUT_FILE"

# Entry
ENTRY_LINES=0
if [ -f "src/main.rs" ]; then
    ENTRY_LINES=$(wc -l < src/main.rs)
fi
echo "Entry Point:                          $ENTRY_LINES lines" >> "$OUTPUT_FILE"

echo '```' >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

echo "### Totals" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"

SRC_LINES=$(find src -name "*.rs" -exec cat {} \; 2>/dev/null | wc -l)
TEST_LINES=$(find tests -name "*.rs" -exec cat {} \; 2>/dev/null | wc -l)
TOTAL_LINES=$((SRC_LINES + TEST_LINES))

echo "Source lines:     $SRC_LINES" >> "$OUTPUT_FILE"
echo "Test lines:       $TEST_LINES" >> "$OUTPUT_FILE"
echo "Total Rust lines: $TOTAL_LINES" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"

echo "### File Counts" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"
SRC_FILES=$(find src -name "*.rs" 2>/dev/null | wc -l | tr -d ' ')
TEST_FILES=$(find tests -name "*.rs" 2>/dev/null | wc -l | tr -d ' ')
echo "Source files: $SRC_FILES" >> "$OUTPUT_FILE"
echo "Test files:   $TEST_FILES" >> "$OUTPUT_FILE"
echo '```' >> "$OUTPUT_FILE"


# ==============================================================================
# LEXICON DATA (FIRST - Reference for all code)
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Lexicon Data

The lexicon defines all vocabulary entries that drive the lexer and parser behavior.

**File:** `assets/lexicon.json`

**Contents:**
- **Keywords** (44 entries): quantifiers, connectives, modals
- **Pronouns** (9 entries): with gender/number/case features
- **Verbs** (252 entries): lemma, Vendler class, irregular forms, features (Ditransitive, SubjectControl, ObjectControl, Raising, Opaque, Factive, Performative, Collective)
- **Nouns** (113 entries): lemma, plural forms, features (Proper, Masculine, Feminine)
- **Adjectives** (65 entries): lemma, features (Intersective, NonIntersective, Gradable)
- **Closed classes**: prepositions, adverbs, scopal/temporal adverbs
- **Morphology rules**: needs_e_ing, needs_e_ed, stemming_exceptions

```json
EOF
cat "assets/lexicon.json" >> "$OUTPUT_FILE"
echo "" >> "$OUTPUT_FILE"
cat >> "$OUTPUT_FILE" << 'EOF'
```

---

EOF

# ==============================================================================
# LEXER & TOKENIZATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Lexer & Tokenization

The lexer transforms English text into a stream of classified tokens using dictionary lookups and heuristic fallbacks for unknown words.

**Location:** `src/token.rs`, `src/lexer.rs`

EOF

add_file "src/token.rs" \
    "Token Definitions" \
    "Token type taxonomy including quantifiers, modal operators, connectives (Because for causal), pronouns, prepositions, demonstratives (This/That/These/Those), Reciprocal (each other), and performatives. Supports presupposition triggers, focus particles, and measure words (MeasureKind: Much/Little). Number(Symbol) stores numeric literals as interned strings for prover-ready symbolic math. Includes semantic token sets (WH_WORDS, MODALS) as const arrays for pattern matching. Span struct (start/end byte positions) for source location tracking. **Phase 12:** TokenType::Ambiguous { primary: Box<TokenType>, alternatives: Vec<TokenType> } for polysemous words that have multiple valid interpretations (e.g., 'duck' as Noun or Verb)."

add_file "src/lexer.rs" \
    "Lexer Implementation" \
    "Dictionary-based tokenization with heuristic word classification. **Verb-First Priority:** Word classification checks verbs before nouns (lines 573-594), enabling the parser safety net where consume_content_word() accepts Verb tokens in noun positions. Disambiguation: words in disambiguation_not_verbs that are also common nouns return Noun; otherwise Adjective. **Verb/Adjective Ambiguity:** Extended ambiguity detection to include Verb AND Adjective overlap (e.g., 'open'); returns TokenType::Ambiguous{Verb, [Adj]} for words that can be either. **Content Word Classifiers:** Heuristic helpers is_noun_like(), is_verb_like(), is_adjective_like() for disambiguating unknown words. **Capitalized Article Disambiguation:** Sentence-initial 'A'/'An' uses lookahead heuristics: followed by logical keyword (if, and, or) → ProperName; followed by verb (not gerund) → ProperName; followed by noun/adjective or lowercase word → Article(Indefinite). Examples: 'A dog ran.' → Article; 'A if B.' → ProperName; 'A red ball.' → Article. Handles contractions, punctuation, unknown word fallbacks, gerund detection (-ing forms as nouns), and mass noun quantifiers (much/little). Enhanced number recognition with word_to_number() for spelled-out numerals and lookahead for compound numbers (twenty five, two and a half). Returns Number(Symbol) tokens for prover-ready symbolic math. UTF-8 safe byte position tracking via char_indices() for span generation. **Contraction Expansion:** Negative contractions split to separate tokens: don't→do+not, doesn't→does+not, didn't→did+not, won't→will+not, can't→cannot. Uses skip_count for character skipping after expansion."

# ==============================================================================
# PARSER & AST
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Parser & AST

The parser builds an Abstract Syntax Tree from the token stream using recursive descent with operator precedence handling. The AST is split into two modules: declarative logic expressions and imperative statements.

**Location:** `src/ast/` (module), `src/parser/`

EOF

add_file "src/ast/mod.rs" \
    "AST Module" \
    "Module exports for the dual-AST architecture. Re-exports logic.rs (declarative) and stmt.rs (imperative) types."

add_file "src/ast/logic.rs" \
    "Logic AST (Declarative)" \
    "Arena-allocated AST with Copy semantics for first-order logic. Boxed large variants (CategoricalData, RelationData, NeoEventData) reduce Expr size from 112 to 32 bytes. Includes compile-time size assertions. **Expression types:** Predicate, Identity, Quantifier (with Generic and island_id for scope constraints), Modal, Temporal, Aspectual, NeoEvent (thematic roles), Event, Control (raising/control verbs), Presupposition, Focus, SpeechAct, Imperative, Comparative (with difference field for measure phrases), Superlative, Counterfactual, Distributive, Scopal, TemporalAnchor, Causal, Intensional (opaque verb wrapper). **Term types:** Constants, Variables, Functions, Sigma, Group, Possessed, Intension (Montague up-arrow ^P for de dicto), Proposition (sentential complement), Value (numeric with kind/unit/dimension). **Intensionality Support:** Term::Intension(Symbol) for de dicto readings; Expr::Intensional { operator, content } for opaque verb contexts. **Sentential Complements:** Term::Proposition(&Expr) wraps embedded clauses as term arguments for verbs like 'say', 'believe', 'think'. Transpiles to bracket notation [expr]. **Scope Tracking:** Expr::Quantifier.island_id: u32 identifies scope boundaries for constraining quantifier movement. **Degree Semantics (Phase 8):** Dimension enum (Length, Time, Weight, Temperature, Cardinality) for measurement categories. NumberKind enum (Real, Integer, Symbolic) for prover-ready numeric types. Term::Value stores numeric value with optional unit Symbol and Dimension. Expr::Comparative.difference field holds optional measure phrase ('2 inches' in 'taller'). ThematicRole enum: Agent, Patient, Theme, Goal, Source, Recipient, Instrument, Location, Time, Manner. VoiceOperator enum (Passive) for voice handling. AspectOperator enum (Progressive, Perfect, Habitual, Iterative) for grammatical aspect. Habitual for present-tense non-stative verbs; Iterative for progressive semelfactives."

add_file "src/ast/stmt.rs" \
    "Statement AST (Imperative)" \
    "Imperative AST for executable code blocks. **Stmt enum variants:** Let (variable binding), Set (mutation), Call (function invocation), If (conditional with then/else blocks), While (loops), Return (with optional value), Assert (bridge to logic kernel - embeds Expr for verification), Give (ownership transfer/move semantics), Show (immutable borrow). **Expr enum (imperative):** Literal (Number, Text, Boolean, Nothing), Identifier, BinaryOp (arithmetic and comparison), Call, Index, Slice. **BinaryOpKind:** Add, Subtract, Multiply, Divide, Eq, NotEq, Lt, Gt, LtEq, GtEq. The Assert statement connects imperative code to the declarative logic kernel, enabling runtime verification via debug_assert! macros in generated Rust."

add_file "src/parser/mod.rs" \
    "Parser Core" \
    "Core Parser struct with token stream, cursor, and context management. **Topicalization:** Detects 'NP + Comma' pattern at sentence start (lines 401-473), stores fronted NP, injects as object with adjective preservation via wrap_with_definiteness_full(). Handles pronoun subjects ('The book, he read.') and full NP subjects ('The apple, John ate.'). **Filler-Gap:** filler_gap: Option<Symbol> field tracks wh-fillers across clause boundaries for long-distance dependencies in relative clauses and wh-questions. **Garden Path Optimization:** Skips reanalysis when auxiliary is present (pending_time.is_some()) since auxiliaries disambiguate structure. ParserGuard RAII struct with guard()/commit() pattern and Deref for transparent parser access with automatic rollback. Entry point for recursive descent parsing. **VP Ellipsis Support:** EventTemplate struct stores verb + non-agent thematic roles + modifiers. capture_event_template() extracts template at NeoEvent creation. last_event_template field persists template for cross-sentence reconstruction. **Phase 12 Parse Forest:** noun_priority_mode: bool field enables lexical ambiguity forking. set_noun_priority_mode() toggles noun-first interpretation for Ambiguous tokens. check_pronoun() respects noun_priority_mode for possessive pronoun handling ('her' as determiner vs object). **Copula Adjective Preference:** After copula (is/was/are/were), simple-aspect Verbs with Adjective alternative prefer Adjective reading via prefer_adjective check (lines 870-884). E.g., 'The door is open' → Adjective(open) rather than Verb. **NPI Handling (Phase 15):** check_npi_quantifier() detects anything/anyone/nobody/nothing; check_npi_object() handles NPI objects in negative contexts; check_temporal_npi() handles ever/never; parse_npi_quantified() produces appropriate quantifier structure based on licensing."

add_file "src/parser/clause.rs" \
    "ClauseParsing Trait" \
    "Extension trait for sentence-level parsing: conditionals (if/then), conjunctions (and/or/but), relative clauses (who/that/which), gapped clauses (ellipsis via verb borrowing), counterfactual antecedents/consequents. Handles complete clause detection and verb extraction. **VP Ellipsis:** try_parse_ellipsis() detects pattern: Subject + Auxiliary (does/do/can/could/would/may/must/should) + (not)? + Terminator (too/also/period/EOF). Reconstructs NeoEvent with new Agent but preserves verb and non-agent roles from last_event_template. Applies modal wrapper and negation as needed."

add_file "src/parser/quantifier.rs" \
    "QuantifierParsing Trait" \
    "Extension trait for quantified expressions: universal (all/every/each), existential (some/a/an), generic (bare plurals), negative (no/none). Handles restrictions, verb phrase parsing for restrictions, definiteness wrapping (with adjectives and PPs), donkey anaphora binding, PP placeholder substitution, and stacked relative clauses ('the book that John read that Mary wrote')."

add_file "src/parser/verb.rs" \
    "VerbParsing Trait" \
    "Extension trait for predicate parsing: subject-verb agreement, aspect chains (progressive/perfect/passive), control structures (want to, try to, seem to), plural subject coordination, thematic role assignment for Neo-Davidsonian events, ditransitive verbs (give/send/tell with Recipient role). **Embedded Clauses:** When filler_gap is set and a verb follows a noun phrase, wraps subordinate clause in Term::Proposition and passes as argument. Enables 'Who did John say Mary loves?' with structure Say(J, [Love(M, x)]). **Do-Support:** Handles do/does/did + (not)? + verb patterns for emphasis and negation. **Sluicing:** Detects wh-word at sentence boundary after embedding verbs (know, wonder); reconstructs event from last_event_template with wh-variable as Agent (who) or Theme (what); wraps in Expr::Question."

add_file "src/parser/noun.rs" \
    "NounParsing Trait" \
    "Extension trait for noun phrase parsing: articles, intersective/non-intersective adjectives (with compound interning), proper names, possessives ('s and 'of' forms), and PP attachment. Includes check_proper_name_or_label() for compound identifiers (set_A, function_F). Registers definite NPs for anaphora with gender/number inference. Provides parse_noun_phrase_for_relative() for relative clause contexts. Converts NounPhrase to Term for predicate arguments."

add_file "src/parser/question.rs" \
    "QuestionParsing Trait" \
    "Extension trait for interrogatives: wh-questions (who/what/where/when/why/how), pied-piping prepositions, yes/no questions with auxiliary inversion, modal-to-vector conversion for question semantics."

add_file "src/parser/modal.rs" \
    "ModalParsing Trait" \
    "Extension trait for modal expressions: necessity/possibility (must/can/might/would/should/cannot), aspect chains (parse_aspect_chain() and parse_aspect_chain_with_term() for perfect/progressive/passive/modal stacking with constant or variable subjects), modal vector construction (domain + force). **Passive Agent Extraction:** Detects 'by X' after passive 'been' to extract agent argument for proper thematic role assignment. **NeoEvent Output:** Creates Expr::NeoEvent with thematic roles for consistent event semantics; adds tense modifiers from pending_time. All modals route through aspect chain parsing for uniform handling of negation and auxiliaries."

add_file "src/parser/pragmatics.rs" \
    "PragmaticsParsing Trait" \
    "Extension trait for pragmatic phenomena: focus particles (only/even/just), measure expressions (much/little), presupposition triggers (factive verbs, aspectual verbs with gerund complement check via is_followed_by_gerund()), scopal adverbs, comparatives (taller than with optional difference measure phrase), superlatives (tallest). parse_measure() handles numeric measurement phrases and routes to comparative parsing when degree expressions are detected."

add_file "src/parser/common.rs" \
    "Parser Constants" \
    "Shared constants for parser modules. COPULAS array defines copular verbs (is/are/was/were) for pattern matching."

add_file "src/parser/tests.rs" \
    "Parser Unit Tests" \
    "Unit tests for parser internals: ParserGuard RAII behavior (guard_restores_all_fields_on_drop), check_any() semantic token matching. Tests verify checkpoint/restore mechanics and token set operations."

# ==============================================================================
# TRANSPILATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Transpilation

The transpiler converts the AST into formal logical notation, supporting both Unicode mathematical symbols and LaTeX output.

**Location:** `src/transpile.rs`, `src/formatter.rs`, `src/registry.rs`

EOF

add_file "src/transpile.rs" \
    "Code Generation" \
    "Converts AST to logical notation. Implements symbolic substitution, quantifier formatting, output mode selection (Unicode/LaTeX), Recipient thematic role rendering, and Causal expression transpilation. Term::Value output formats numeric values (Real/Integer/Symbolic) with optional unit strings. Comparative.difference renders measure phrases in degree expressions. Includes write_to() and write_logic() methods for zero-allocation output to any std::fmt::Write target."

add_file "src/formatter.rs" \
    "Output Formatting" \
    "LatexFormatter, UnicodeFormatter, and LogicFormatter traits. Handles symbol sanitization and operator rendering for clean output."

add_file "src/registry.rs" \
    "Symbol Registry" \
    "Maps interned symbols to readable output strings. Manages predicate and constant naming conventions."

# ==============================================================================
# SEMANTIC ANALYSIS
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Semantic Analysis

Advanced semantic computation using lambda calculus for compositional meaning construction.

**Location:** `src/lambda.rs`, `src/context.rs`, `src/view.rs`

EOF

add_file "src/lambda.rs" \
    "Lambda Calculus" \
    "Lambda calculus core with Montague-style compositional semantics. Features: Lambda abstraction, application, and beta reduction. **Quantifier Scope Enumeration** via enumerate_scopings() returning lazy ScopeIterator. **Complexity:** Factorial O(n!) worst-case, optimized by Island constraints to Π(k_i!). **Island Constraints:** Scope boundaries (if/and/or) prevent cross-island quantifier movement. **Intensionality (De Re/De Dicto):** enumerate_intensional_readings() for opaque verbs (seek, want, believe, need, fear). **Opacity-Respecting Substitution:** substitute_respecting_opacity() blocks substitution inside intensional contexts. **Montague Up-Arrow:** Term::Intension(^P) for de dicto readings."

add_file "src/context.rs" \
    "Discourse Context" \
    "Entity registration and resolution for anaphora. Tracks gender, number, and case attributes for pronoun binding."

add_file "src/view.rs" \
    "AST Views & Resolution" \
    "ExprView (including Causal variant), TermView, NounPhraseView types for AST traversal. Symbol resolution and display utilities."

add_file "src/semantics/mod.rs" \
    "Semantics Module" \
    "Entry point for semantic axiom layer. Includes generated axiom_data and exports apply_axioms()."

add_file "src/semantics/axioms.rs" \
    "Axiom Expansion" \
    "AST transformation for meaning postulates. Handles noun entailments (bachelor→unmarried), hypernyms (dog→animal), privative adjectives (fake→¬N∧Resembles), and verb entailments (murder→kill)."

# ==============================================================================
# TYPE ANALYSIS (TWO-PASS COMPILATION)
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Type Analysis

Two-pass compilation infrastructure for type discovery and resolution.

**Location:** `src/analysis/`

EOF

add_file "src/analysis/mod.rs" \
    "Analysis Module" \
    "Entry point for type analysis. Re-exports TypeRegistry and DiscoveryPass for two-pass compilation."

add_file "src/analysis/registry.rs" \
    "Type Registry" \
    "TypeRegistry struct for tracking type definitions. TypeDef enum with variants: Generic (type parameters), Struct (record types), Enum (sum types). register_type() adds definitions; resolve_type() looks up by name. Supports the Adjective System where adjectives become type parameters."

add_file "src/analysis/discovery.rs" \
    "Discovery Pass" \
    "First pass of two-pass compilation. DiscoveryPass scans source for ## Definition blocks to populate TypeRegistry before full parsing. Enables forward references and mutual recursion in type definitions. Extracts type names, parameters, and kind (struct/enum) from definition headers."

# ==============================================================================
# CODE GENERATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Code Generation

Rust code emission from imperative AST.

**Location:** `src/codegen.rs`, `src/compile.rs`, `src/scope.rs`

EOF

add_file "src/codegen.rs" \
    "Rust Code Generation" \
    "Converts imperative Stmt AST to valid Rust source code. codegen_program() emits complete program with main(). codegen_stmt() handles each Stmt variant: Let→let binding, Set→assignment, Call→function call, If→if/else, While→while loop, Return→return, Assert→debug_assert!, Give→move semantics, Show→borrow. codegen_expr() handles imperative expressions. Uses String buffer for zero-dependency output."

add_file "src/compile.rs" \
    "Compilation Orchestration" \
    "High-level compilation pipeline. compile_to_rust() coordinates lexer→parser→codegen for imperative programs. Manages parser mode switching between declarative and imperative contexts. Handles ## Main and ## Definition block routing."

add_file "src/scope.rs" \
    "Scope Management" \
    "Variable scope tracking for imperative blocks. ScopeStack manages nested lexical scopes with push/pop. resolve_identifier() finds variable bindings respecting shadowing. Tracks ownership state (owned/moved/borrowed) for each binding."

# ==============================================================================
# SUPPORT INFRASTRUCTURE
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Public API

The public interface for embedding LOGICAFFEINE in other applications.

**Location:** `src/lib.rs`

EOF

add_file "src/lib.rs" \
    "Library Entry Point" \
    "Exports compile(), compile_with_options(), compile_ambiguous(), compile_all_scopes(), compile_all_scopes_with_options(), compile_discourse(), compile_discourse_with_options(), and compile_with_context(). **Phase 12 Parse Forest:** compile_forest() and compile_forest_with_options() return Vec<String> of all valid readings for ambiguous sentences. MAX_FOREST_READINGS (12) limits output size. Handles lexical ambiguity (Noun/Verb tokens) via noun_priority_mode forking and structural ambiguity (PP attachment) via pp_attachment_mode. **Ambiguity APIs:** compile_ambiguous() returns Vec<String> for PP-attachment ambiguity; compile_all_scopes() returns all quantifier scope readings PLUS intensional readings (de re/de dicto) by calling enumerate_scopings() for scope permutations then enumerate_intensional_readings() for opaque verb ambiguity. **Discourse API:** compile_discourse() handles multi-sentence input with persistent DiscourseContext. Defines TranspileContext, CompileOptions, and OutputFormat (Unicode/LaTeX)."

cat >> "$OUTPUT_FILE" << 'EOF'
## Linguistic Data

Dictionary and semantic information for word classification.

**Location:** `src/lexicon.rs`

EOF

add_file "src/lexicon.rs" \
    "Lexicon" \
    "Feature-based lexical database. Feature enum (22 variants) classifies words by transitivity (Transitive, Intransitive, Ditransitive), control theory (SubjectControl, ObjectControl, Raising), semantics (Opaque, Factive, Performative, Collective), noun properties (Count, Mass, Proper, Masculine, Feminine, Neuter, Animate, Inanimate), and adjective types (Intersective, Subsective, NonIntersective, Gradable). Subsective adjectives use intension (^Noun) for class-relative predicates like 'small elephant' → S(x, ^E). VerbClass enum implements Vendler's Aktionsart with is_stative()/is_durative()/is_telic(). Metadata structs (VerbMetadata, NounMetadata, AdjectiveMetadata) provide lemma plus feature arrays. Generated lookup functions (lookup_verb_db, lookup_noun_db, lookup_adjective_db) return full metadata at runtime. is_subsective() generated check for adjective type. **Zero-Derivation Morphology:** Consonant cluster heuristic (vowel + consonant + l/r) recovers silent-e lemmas: 'tabled' → 'table', 'googled' → 'google'. **Phase 11 Sort System:** Sort enum (Human, Animate, Celestial, Abstract, Physical, Value) for ontological type hierarchy. lookup_sort() returns Sort for proper names. is_compatible_with() checks sort subsumption (Human⊂Animate⊂Physical). Used for metaphor detection via sort violations ('Juliet is the sun' violates Human/Celestial compatibility)."

add_file "src/mwe.rs" \
    "Multi-Word Expressions" \
    "Post-tokenization MWE pipeline (Phase 13). MweTrie for pattern storage with longest-match lookup. apply_mwe_pipeline() collapses multi-token sequences into single semantic units. Handles compound nouns (fire engine → FireEngine), idioms (kicked the bucket → Die), and phrasal verbs (gave up → Surrender). Inherits tense from head token for morphological variants. build_mwe_trie() creates default vocabulary with common MWEs."

add_file "src/ontology.rs" \
    "Ontology Module" \
    "Bridging anaphora and sort checking (Phase 14). find_bridging_wholes() returns possible whole objects for parts (e.g., 'engine' → ['car', 'plane']). check_sort_compatibility() validates predicate-subject sort match for metaphor detection. required_sort() gets predicate's required sort. Uses generated ontology_data.rs from build.rs with part-whole mappings and predicate sort requirements."

cat >> "$OUTPUT_FILE" << 'EOF'
## Memory Management

Efficient memory allocation strategies for AST construction.

**Location:** `src/intern.rs`, `src/arena.rs`

EOF

add_file "src/intern.rs" \
    "Symbol Interning" \
    "Interner and Symbol types for efficient string storage. Enables O(1) symbol comparisons and reduced memory footprint."

add_file "src/arena.rs" \
    "Arena Allocation" \
    "Bumpalo-based arena allocator for AST nodes. Provides fast allocation with batch deallocation."

add_file "src/arena_ctx.rs" \
    "AST Context" \
    "AstContext struct unifying 6 separate arenas into one Copy struct. Provides alloc_expr(), alloc_term(), alloc_slice() helpers for ergonomic AST construction. Fluent expression builders: binary(), unary(), quantifier(), temporal(), aspectual(), modal() with #[inline(always)]."

cat >> "$OUTPUT_FILE" << 'EOF'
## Error Handling

User-friendly error reporting with educational feedback.

**Location:** `src/error.rs`

EOF

add_file "src/error.rs" \
    "Error Types" \
    "ParseError and ParseErrorKind types. ParseError includes Span for source location. display_with_source() renders errors with ANSI colors, line numbers, underline markers, and 'did you mean?' suggestions. Implements socratic_explanation() for Socratic-style error guidance."

cat >> "$OUTPUT_FILE" << 'EOF'
## Suggestions & Styling

Compiler-style error presentation with typo correction and ANSI colors.

**Location:** `src/suggest.rs`, `src/style.rs`

EOF

add_file "src/suggest.rs" \
    "Typo Suggestions" \
    "Zero-dependency Levenshtein distance algorithm. find_similar() finds closest vocabulary match for 'did you mean?' suggestions in error messages."

add_file "src/style.rs" \
    "ANSI Styling" \
    "Style struct with red(), blue(), cyan(), green(), bold_red() methods for terminal color output. Integrated into display_with_source() for compiler-style error presentation."

cat >> "$OUTPUT_FILE" << 'EOF'
## Debug Utilities

Development and introspection tools.

**Location:** `src/debug.rs`

EOF

add_file "src/debug.rs" \
    "Debug Tools" \
    "DebugWorld for AST introspection. DisplayWith trait for custom formatting during development. Includes Causal expression display support."

cat >> "$OUTPUT_FILE" << 'EOF'
## Visitor Pattern

Tree traversal infrastructure for AST analysis.

**Location:** `src/visitor.rs`

EOF

add_file "src/visitor.rs" \
    "Visitor Trait" \
    "Visitor trait with walk_expr() and walk_term() functions for AST traversal. Enables analysis passes without manual recursion."

cat >> "$OUTPUT_FILE" << 'EOF'
## Test Utilities

Helper functions for unit and integration testing.

**Location:** `src/test_utils.rs`

EOF

add_file "src/test_utils.rs" \
    "Test Helpers" \
    "Utility functions for constructing test cases and validating transpilation output. assert_snapshot! macro for golden master testing. Snapshots stored in tests/snapshots/. Set UPDATE_SNAPSHOTS=1 to regenerate."

cat >> "$OUTPUT_FILE" << 'EOF'
## Pragmatics

Speech act theory and modal-to-imperative conversion.

**Location:** `src/pragmatics.rs`

EOF

add_file "src/pragmatics.rs" \
    "Pragmatics Module" \
    "Modal-to-imperative conversion for indirect speech acts. Detects when modal questions should be interpreted as imperatives (e.g., 'Can you pass the salt?' → Pass(you, salt), 'Could you please open the door?' → Open(you, door)). Handles both Expr::NeoEvent and Expr::Predicate forms for addressee detection."

# ==============================================================================
# GAMIFICATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Gamification

Achievement system, progress tracking, and spaced repetition for learning engagement.

**Location:** `src/achievements.rs`, `src/progress.rs`, `src/game.rs`, `src/srs.rs`

EOF

add_file "src/achievements.rs" \
    "Achievements" \
    "Achievement system with unlock conditions and tracking. Defines achievements for milestones (first problem, streak, mastery). Checks unlock conditions and emits events for UI notifications."

add_file "src/progress.rs" \
    "Progress Tracking" \
    "Lesson and module progress tracking. Tracks completion status, scores, and time spent. Persists progress to storage for cross-session continuity."

add_file "src/game.rs" \
    "Game State" \
    "Central game state management. Tracks XP, level, combo/streak counters, and current lesson. Coordinates between achievements, progress, and SRS systems."

add_file "src/srs.rs" \
    "Spaced Repetition" \
    "SM-2 style spaced repetition algorithm for review scheduling. Calculates next review date based on performance. Prioritizes due items in review queue."

add_file "src/audio.rs" \
    "Audio Feedback" \
    "Sound effect management for feedback. Plays success/failure/achievement sounds. Uses web audio API in WASM context."

add_file "src/storage.rs" \
    "Persistent Storage" \
    "LocalStorage interface for saving game state. Handles serialization/deserialization of progress, settings, and achievements. Provides fallback for browsers without storage access."

# ==============================================================================
# APPLICATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Entry Point

Command-line interface and REPL for interactive use.

**Location:** `src/main.rs`

EOF

add_file "src/main.rs" \
    "Application Entry Point" \
    "Web application entry point. Launches Dioxus web UI with Router for SPA navigation. Build with 'dx serve' for development or 'dx build' for production WASM deployment."

cat >> "$OUTPUT_FILE" << 'EOF'
## Web Application

Dioxus-based web application with routing and multiple pages.

**Location:** `src/ui/`

**Architecture:**
- Router-based SPA with client-side navigation
- Pages: Home (Quadrivium menu), Workspace (chat interface), Pricing, Learn (curriculum browser), Lesson (problem-solving)
- Components: Reusable UI elements (chat display, input area)
- Problem Generator: Template-based exercise generation with semantic grading
- Fair Source licensing with honor system toggle

EOF

if [ -d "src/ui" ]; then
    for file in src/ui/*.rs; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            case "$filename" in
                app.rs)
                    add_file "$file" "UI: App" "Root application component with Router wrapper and global CSS styles (gradients, glassmorphism, animations)."
                    ;;
                router.rs)
                    add_file "$file" "UI: Router" "Dioxus Router with routes: / (Home), /pricing (Pricing), /studio (Studio), /learn (Learn), /lesson/:era/:module (Lesson), /workspace/:subject (Workspace), /:..route (NotFound 404 handler). Includes NotFound component for graceful 404 handling."
                    ;;
                state.rs)
                    add_file "$file" "UI: State" "Application state management with Signal-based reactivity. ChatMessage history and compile integration."
                    ;;
                *)
                    add_file "$file" "UI: ${filename%.rs}" "UI module built with Dioxus 0.6."
                    ;;
            esac
        fi
    done

    if [ -d "src/ui/pages" ]; then
        for file in src/ui/pages/*.rs; do
            if [ -f "$file" ]; then
                filename=$(basename "$file")
                case "$filename" in
                    home.rs)
                        add_file "$file" "Page: Home" "Quadrivium landing page with 4 subject portals (Logic, English, Coding, Mathematics) and Fair Source license banner with honor system toggle."
                        ;;
                    workspace.rs)
                        add_file "$file" "Page: Workspace" "Three-column learning interface: sidebar (lesson tree, history), center (chat/proof interface), right panel (AST inspector)."
                        ;;
                    pricing.rs)
                        add_file "$file" "Page: Pricing" "Commercial licensing information page with Fair Source explanation and enterprise contact details."
                        ;;
                    learn.rs)
                        add_file "$file" "Page: Learn" "Curriculum browser with expandable era/module hierarchy. Displays Trivium, Quadrivium, and Metaphysics eras with nested modules."
                        ;;
                    lesson.rs)
                        add_file "$file" "Page: Lesson" "Interactive problem-solving interface. Displays generated challenges, accepts FOL input, provides semantic grading with feedback, and tracks progress through exercises."
                        ;;
                    studio.rs)
                        add_file "$file" "Page: Studio" "Live transpilation sandbox with AST visualization, portal animations, and real-time English-to-FOL conversion. Header nav uses Link components for client-side routing to Home and Learn pages."
                        ;;
                    mod.rs)
                        add_file "$file" "Pages: Module" "Page module exports for Home, Pricing, Workspace, Learn, Lesson, and Studio pages."
                        ;;
                    *)
                        add_file "$file" "Page: ${filename%.rs}" "Application page component."
                        ;;
                esac
            fi
        done
    fi

    if [ -d "src/ui/components" ]; then
        for file in src/ui/components/*.rs; do
            if [ -f "$file" ]; then
                filename=$(basename "$file")
                case "$filename" in
                    chat.rs)
                        add_file "$file" "Component: ChatDisplay" "Renders chat message history with role-based styling (user, system, error)."
                        ;;
                    input.rs)
                        add_file "$file" "Component: InputArea" "Text input with Enter key submission and Transpile button."
                        ;;
                    *)
                        add_file "$file" "Component: ${filename%.rs}" "Reusable UI component."
                        ;;
                esac
            fi
        done
    fi
fi

# ==============================================================================
# PROBLEM GENERATOR
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Problem Generator

The Problem Generator transforms LOGOS from a sandbox into an interactive teaching tool with curriculum-based exercises.

**Location:** `src/content.rs`, `src/generator.rs`, `src/grader.rs`, `src/runtime_lexicon.rs`

**Architecture:**
```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Problem Generator Pipeline                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   ┌───────────┐    ┌───────────┐    ┌───────────┐    ┌───────────┐        │
│   │ Curriculum│───▶│ Generator │───▶│ Challenge │───▶│  Grader   │        │
│   │   JSON    │    │  Engine   │    │           │    │           │        │
│   └───────────┘    └─────┬─────┘    └───────────┘    └─────┬─────┘        │
│                          │                                  │              │
│                          ▼                                  ▼              │
│                    ┌───────────┐                      ┌───────────┐        │
│                    │  Runtime  │                      │ Semantic  │        │
│                    │  Lexicon  │                      │ Equality  │        │
│                    └───────────┘                      └───────────┘        │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Curriculum Structure

Filesystem-based curriculum organization embedded at compile time via `include_dir`.

```
assets/curriculum/
├── 01_trivium/                    # Era I: Naming
│   ├── meta.json                  # Era metadata (id, title, description)
│   ├── 01_atomic/                 # Module: Predication
│   │   ├── meta.json              # Module metadata (pedagogy, order)
│   │   ├── ex_01_adjectives.json  # Exercise: {ProperName} is {Adjective}
│   │   └── ex_02_intransitive.json
│   ├── 02_relations/              # Module: Transitive verbs
│   └── 03_negation/               # Module: Negation
├── 02_quadrivium/                 # Era II: Quantification
│   ├── 01_universal/              # ∀x patterns
│   ├── 02_existential/            # ∃x patterns
│   └── 03_scope/                  # Scope ambiguity
└── 03_metaphysics/                # Era III: Modality & Time
    ├── 01_modality/               # □ and ◇ operators
    └── 02_time/                   # Past and Future operators
```

**Exercise Schema:**
```json
{
  "id": "ex_01",
  "type": "translation",
  "difficulty": 1,
  "prompt": "Translate this observation:",
  "template": "{ProperName} is {Adjective}.",
  "constraints": { "Adjective": ["Intersective"] },
  "hint": "Apply the adjective as a predicate to the constant."
}
```

### Template Slots

| Slot | Example | Constraints |
|------|---------|-------------|
| `{ProperName}` | John, Mary | Proper nouns from lexicon |
| `{Noun}` | dog, cat | Common nouns, filterable by sort |
| `{Noun:Plural}` | dogs, cats | Plural form of common noun |
| `{Verb}` | runs, sleeps | Intransitive verbs |
| `{Verb:Past}` | ran, slept | Past tense form |
| `{Adjective}` | happy, tall | Intersective by default |

### Semantic Grading

The grader performs semantic equivalence checking, not string matching:

1. **Unicode normalization**: `\forall` → `∀`, `->` → `→`, `&` → `∧`
2. **Whitespace removal**: `∀x ( P(x) )` → `∀x(P(x))`
3. **Commutativity**: `P ∧ Q` equals `Q ∧ P`
4. **Structural similarity**: Partial credit for close attempts

**Grading Results:**
| Score | Meaning |
|-------|---------|
| 100 | Correct (semantically equivalent) |
| 35-50 | Partial (close structure) |
| 0 | Incorrect |

EOF

add_file "src/content.rs" \
    "Content Engine" \
    "Loads curriculum from embedded JSON files. Uses include_dir to embed assets/curriculum/ at compile time. Provides ContentEngine for querying eras, modules, and exercises."

add_file "src/generator.rs" \
    "Generator Engine" \
    "Template-based problem generation. Fills slots like {ProperName}, {Verb}, {Adjective} using runtime lexicon queries with constraint filtering. Applies morphological transforms for modifiers like :Plural and :Past."

add_file "src/grader.rs" \
    "Answer Grader" \
    "Semantic equivalence checking for FOL answers. Normalizes Unicode, handles commutativity of ∧/∨, and provides partial credit scoring. Uses structural AST comparison after normalization."

add_file "src/runtime_lexicon.rs" \
    "Runtime Lexicon" \
    "Runtime access to lexicon data for the generator. Provides query APIs: nouns_with_feature(), verbs_with_feature(), nouns_with_sort(), proper_nouns(), common_nouns(). Loads from embedded lexicon.json."

# ==============================================================================
# LOGOS CORE RUNTIME
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Logos Core Runtime

Embedded runtime library for compiled LOGOS programs. Provides type aliases and IO functions per the Spec.

**Location:** `logos_core/src/`

EOF

add_file "logos_core/src/lib.rs" \
    "Runtime Library" \
    "Entry point for logos_core crate. Re-exports io, types, and prelude modules. Embedded into compiled programs via include_str! in src/compile.rs."

add_file "logos_core/src/types.rs" \
    "Type Aliases" \
    "Rust type aliases per Spec §10.6.1: Nat→u64, Int→i64, Real→f64, Text→String, Bool→bool, Unit→()."

add_file "logos_core/src/io.rs" \
    "IO Functions" \
    "Standard IO per Spec §10.5: show() for display, read_line() for input, println/eprintln/print for output."

# ==============================================================================
# ==============================================================================
# DYNAMIC FILE DISCOVERY
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Additional Modules

Any additional source files not explicitly categorized above.

EOF

for file in src/*.rs; do
    if [ -f "$file" ]; then
        case "$file" in
            */lib.rs|*/main.rs|*/token.rs|*/lexer.rs|*/ast.rs|*/parser.rs|\
            */transpile.rs|*/formatter.rs|*/registry.rs|*/lambda.rs|\
            */context.rs|*/view.rs|*/lexicon.rs|*/intern.rs|*/arena.rs|\
            */arena_ctx.rs|*/visitor.rs|*/suggest.rs|*/style.rs|\
            */error.rs|*/debug.rs|*/test_utils.rs|*/pragmatics.rs|\
            */content.rs|*/generator.rs|*/grader.rs|*/runtime_lexicon.rs)
                continue
                ;;
        esac
        filename=$(basename "$file")
        add_file "$file" \
            "Module: ${filename%.rs}" \
            "Additional source module."
    fi
done

# ==============================================================================
# CARGO.TOML
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Build Configuration

EOF

add_file "Cargo.toml" \
    "Package Manifest" \
    "Rust package configuration with dependencies: bumpalo (arena allocator), dioxus (desktop UI)."

add_file "build.rs" \
    "Build Script" \
    "Generates compile-time lookup functions from lexicon.json. Expands verbs into irregular verb entries and feature-based VerbDbEntry records. Derives behavioral lists (is_ditransitive_verb, is_subject_control_verb, is_object_control_verb, is_raising_verb, is_opaque_verb, is_collective_verb, is_performative) from feature arrays. Generates lookup_verb_db(), lookup_noun_db(), lookup_adjective_db() returning metadata with feature slices. Produces is_* check functions for closed classes and morphology rules."

# ==============================================================================

# METADATA
# ==============================================================================
cat >> "$OUTPUT_FILE" << EOF

---

## Metadata

- **Generated:** $(date)
- **Repository:** $(pwd)
- **Git Branch:** $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
- **Git Commit:** $(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
- **Documentation Size:** $(du -h "$OUTPUT_FILE" | cut -f1)

---

**Note:** This documentation is auto-generated. Run \`./generate-docs.sh\` to regenerate after code changes.
EOF

# ==============================================================================
# SUMMARY
# ==============================================================================
echo ""
echo "Documentation generated: $OUTPUT_FILE"
echo ""
echo "Summary:"
echo "--------"
echo "  Source files: $SRC_FILES"
echo "  Test files:   $TEST_FILES"
echo "  Total lines:  $TOTAL_LINES"
echo ""
echo "  Documentation size: $(du -h "$OUTPUT_FILE" | cut -f1)"
echo ""
echo "Done! View with: cat $OUTPUT_FILE"
