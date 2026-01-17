#!/bin/bash

# LOGOS LOGICAL - Core Logic Documentation Generator
# Generates documentation for the English-to-First-Order-Logic transpiler core.

OUTPUT_FILE="LOGOS_LOGICAL_DOCS.md"
echo "Generating LOGICAL documentation..."

# ==============================================================================
# HEADER & TOC
# ==============================================================================
cats > "$OUTPUT_FILE" << 'EOF'
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

EOF

# ==============================================================================
# HELPERS
# ==============================================================================
add_file() {
    local file_path="$1"
    local title="$2"
    local description="$3"
    if [ -f "$file_path" ]; then
        echo "Adding: $file_path"
        {
            echo "### $title"
            echo ""
            echo "**File:** \`$file_path\`"
            echo ""
            echo "$description"
            echo ""
            echo "\
```rust"
            cat "$file_path"
            echo "\
```"
            echo ""
            echo "---"
            echo ""
        } >> "$OUTPUT_FILE"
    fi
}

add_test_description() {
    local file_path="$1"
    local title="$2"
    local description="$3"
    if [ -f "$file_path" ]; then
        echo "Adding test: $file_path"
        {
            echo "#### $title"
            echo ""
            echo "**File:** \`$file_path\`"
            echo ""
            echo "$description"
            echo ""
            echo "---"
            echo ""
        } >> "$OUTPUT_FILE"
    fi
}

# ==============================================================================
# CONTENT
# ==============================================================================

# Lexer
add_file "src/token.rs" "Token Definitions" "Token enum, TokenType, and semantic sets (WH_WORDS, MODALS)."
add_file "src/lexer.rs" "Lexer Implementation" "Tokenization logic, heuristic classification, and disambiguation."

# Parser
add_file "src/ast/logic.rs" "Logic AST" "Declarative AST types: Expr (Predicate, Quantifier, Modal), Term (Constants, Variables)."
add_file "src/parser/mod.rs" "Parser Core" "Parser struct, entry points, and ambiguity handling (Parse Forest)."
add_file "src/parser/clause.rs" "Clause Parsing" "Sentence structure, coordination, and ellipsis."
add_file "src/parser/quantifier.rs" "Quantifier Parsing" "Universal/Existential parsing, scope islands, and restrictions."
add_file "src/parser/verb.rs" "Verb Parsing" "Verb phrases, aspect chains, and event semantics."
add_file "src/parser/noun.rs" "Noun Parsing" "Noun phrases, relative clauses, and appositives."

# Semantics
add_file "src/lambda.rs" "Lambda Calculus" "Scope enumeration, beta reduction, and intensionality handling."
add_file "src/drs.rs" "Discourse Representation" "DRT implementation for anaphora resolution."
add_file "src/semantics/kripke.rs" "Kripke Semantics" "Modal lowering to possible worlds."
add_file "src/pragmatics.rs" "Pragmatics" "Speech acts and implicatures."

# Transpilation
add_file "src/transpile.rs" "Transpiler" "AST to string conversion (Unicode, LaTeX)."

# Ontology & Support
add_file "src/lexicon.rs" "Lexicon" "Word features, Vendler classes, and zero-derivation."
add_file "src/ontology.rs" "Ontology" "Sort system and bridging anaphora."
add_file "src/mwe.rs" "Multi-Word Expressions" "Trie-based MWE collapsing."

# Tests
cats >> "$OUTPUT_FILE" << 'EOF'
## Relevant Tests
EOF

add_test_description "tests/phase1_garden_path.rs" "Phase 1: Garden Path" "Structural reanalysis tests."
add_test_description "tests/phase3_time.rs" "Phase 3: Time" "Reichenbach temporal logic."
add_test_description "tests/phase7_semantics.rs" "Phase 7: Intensionality" "De re/de dicto and opaque contexts."
add_test_description "tests/phase10_ellipsis.rs" "Phase 10: Ellipsis" "VP ellipsis reconstruction."
add_test_description "tests/phase12_ambiguity.rs" "Phase 12: Ambiguity" "Parse forest and lexical ambiguity."
add_test_description "tests/phase_kripke.rs" "Kripke Semantics" "Modal logic possible worlds."

echo "Done! View with: cat $OUTPUT_FILE"
chmod +x "$OUTPUT_FILE"