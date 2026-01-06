# LOGOS - Logical Core Documentation

## Overview
This document covers the **Logical Core** of the LOGOS system: the English-to-FOL transpiler, semantic analysis, and linguistic processing pipeline.

## Table of Contents
1. [Architecture](#architecture)
2. [Grammar & Semantics](#grammar--semantics)
3. [Lexer & Tokenization](#lexer--tokenization)
4. [Parser & AST](#parser--ast)
5. [Semantics & Lambda Calculus](#semantics--lambda-calculus)
6. [Transpilation](#transpilation)
7. [Ontology & MWE](#ontology--mwe)
8. [Relevant Tests](#relevant-tests)

## Architecture

**Pipeline:**
`Input (English) -> Lexer -> Token Stream -> Parser (Recursive Descent) -> AST (Logic) -> Semantics (Lambda/DRT) -> Transpiler -> Output (FOL/Kripke)`

**Key Components:**
*   **Lexer (`lexer.rs`):** Dictionary-based, handles multi-word expressions (MWE), contractions, and part-of-speech disambiguation (Verb-first priority).
*   **Parser (`parser/`):** Recursive descent, produces `ast::logic::Expr`. Handles structural ambiguity (Parse Forest), operator precedence, and linguistic phenomena (Topicalization, Sluicing, Ellipsis).
*   **Semantics (`lambda.rs`, `drs.rs`):** Compositional semantics via Lambda Calculus. Handles quantifier scope (island constraints), intensionality (de re/de dicto), and discourse representation (DRT).
*   **Transpiler (`transpile.rs`):** Converts AST to Unicode, LaTeX, or SimpleFOL strings.

### Token Definitions
**File:** `src/token.rs`

Token enum, TokenType, and semantic sets (WH_WORDS, MODALS).



---
### Lexer Implementation
**File:** `src/lexer.rs`

Tokenization logic, heuristic classification, and disambiguation.



---
### Logic AST
**File:** `src/ast/logic.rs`

Declarative AST types: Expr (Predicate, Quantifier, Modal), Term (Constants, Variables).



---
### Parser Core
**File:** `src/parser/mod.rs`

Parser struct, entry points, and ambiguity handling (Parse Forest).



---
### Clause Parsing
**File:** `src/parser/clause.rs`

Sentence structure, coordination, and ellipsis.



---
### Quantifier Parsing
**File:** `src/parser/quantifier.rs`

Universal/Existential parsing, scope islands, and restrictions.



---
### Verb Parsing
**File:** `src/parser/verb.rs`

Verb phrases, aspect chains, and event semantics.



---
### Noun Parsing
**File:** `src/parser/noun.rs`

Noun phrases, relative clauses, and appositives.



---
### Lambda Calculus
**File:** `src/lambda.rs`

Scope enumeration, beta reduction, and intensionality handling.



---
### Discourse Representation
**File:** `src/drs.rs`

DRT implementation for anaphora resolution.



---
### Kripke Semantics
**File:** `src/semantics/kripke.rs`

Modal lowering to possible worlds.



---
### Pragmatics
**File:** `src/pragmatics.rs`

Speech acts and implicatures.



---
### Transpiler
**File:** `src/transpile.rs`

AST to string conversion (Unicode, LaTeX).



---
### Lexicon
**File:** `src/lexicon.rs`

Word features, Vendler classes, and zero-derivation.



---
### Ontology
**File:** `src/ontology.rs`

Sort system and bridging anaphora.



---
### Multi-Word Expressions
**File:** `src/mwe.rs`

Trie-based MWE collapsing.



---
## Relevant Tests
#### Phase 1: Garden Path
**File:** `tests/phase1_garden_path.rs`
Structural reanalysis tests.
---
#### Phase 3: Time
**File:** `tests/phase3_time.rs`
Reichenbach temporal logic.
---
#### Phase 7: Intensionality
**File:** `tests/phase7_semantics.rs`
De re/de dicto and opaque contexts.
---
#### Phase 10: Ellipsis
**File:** `tests/phase10_ellipsis.rs`
VP ellipsis reconstruction.
---
#### Phase 12: Ambiguity
**File:** `tests/phase12_ambiguity.rs`
Parse forest and lexical ambiguity.
---
#### Kripke Semantics
**File:** `tests/phase_kripke.rs`
Modal logic possible worlds.
---
