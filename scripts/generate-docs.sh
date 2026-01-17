#!/bin/bash

# LOGICAFFEINE 1.0 - Master Documentation Generator
# Generates comprehensive markdown documentation for the LOGOS System.

OUTPUT_FILE="LOGOS_DOCUMENTATION.md"

echo "Generating comprehensive LOGICAFFEINE documentation..."

# ==============================================================================
# HEADER & TABLE OF CONTENTS
# ==============================================================================
catalyst > "$OUTPUT_FILE" << 'EOF'
# LOGICAFFEINE 1.0 - System Documentation

# **Logicaffeine** is a hybrid platform combining a rigorous English-to-First-Order-Logic transpiler (**Logical Core**) with an imperative programming language (**Imperative Layer**) and a gamified learning environment (**Frontend**).

## Table of Contents

### I. System Overview
1. [Architecture & Pipelines](#architecture-overview)
2. [Grammar Rules & Patterns](#grammar-rules)
3. [Project Statistics](#statistics)

### II. Logical Core (Declarative)
4. [Lexicon & Tokenization](#lexicon-data)
5. [Parser & AST](#parser--ast)
6. [Semantics (Lambda/DRT)](#semantic-analysis)
7. [Transpilation (FOL/Kripke)](#transpilation)

### III. Imperative Layer (Executable)
8. [Compiler & Codegen](#code-generation)
9. [Type System & Analysis](#type-analysis)
10. [Runtime & Standard Lib](#logos-core-runtime)
11. [Verification (Z3)](#logos-verification-crate)

### IV. Frontend & Application
12. [Web Application (Dioxus)](#web-application)
13. [Problem Generator](#problem-generator)
14. [Gamification](#gamification)

### V. Reference
15. [Test Suite](#integration-tests)
16. [Public API](#public-api)

---

## Architecture Overview

LOGICAFFEINE consists of two primary compilation pipelines sharing a common frontend (Lexer).

### 1. The Logical Pipeline (English -> Logic)
Translates natural language into formal notation (FOL, LaTeX, Kripke). Handles ambiguity, scope, and semantics.

### 2. The Imperative Pipeline (English -> Rust)
Compiles controlled English into executable Rust code. Handles type checking, memory safety, and concurrency.

```mermaid
graph TD
    A[Input English] --> B(Lexer Tokens);
    B --> C{Parser AST};
    C --> D{Parser Modes};
    D -- Logical Mode --> E[Parse Forest];
    E --> F[Lambda Calculus];
    F --> G[DRT];
    G --> H[Kripke Lowering];
    H --> I[Output Generation];
    D -- Imperative Mode --> J[Stmt AST];
    J --> K[Type Analysis];
    K --> L[Verification Z3];
    L --> M[Code Generation Rust];

    subgraph Logical Pipeline
        E; F; G; H; I;
    end
    subgraph Imperative Pipeline
        J; K; L; M;
    end
```

### Key Technical Features

**Core Engine:**
- **Zero-Copy AST:** `bumpalo` arena allocation for high-performance AST manipulation.
- **Dual AST:** `ast::logic` (Declarative) and `ast::stmt` (Imperative) sharing common `Term` structures.
- **Parse Forest:** Handles structural ambiguity (PP-attachment, scope) by producing all valid readings.
- **Lexical Priority:** Verb-first disambiguation with safety nets for Noun/Verb polysemy ("Time flies").

**Logical Features:**
- **Neo-Davidsonian Events:** `∃e(Kick(e) ∧ Agent(e,j))` semantics.
- **Montague Semantics:** Compositional lambda calculus.
- **Generalized Quantifiers:** `MANY`, `MOST`, `FEW` alongside `∀`, `∃`.
- **Intensionality:** De Re/De Dicto distinction for opaque verbs (`seek`, `want`).

**Imperative Features:**
- **Ownership System:** Linear types modeled on Rust ownership (Move/Borrow).
- **Refinement Types:** `Let x: Int where x > 0` validated by Z3 or runtime checks.
- **Concurrency:** Structured concurrency (`Simultaneously:`) and Go-like channels (`Pipe`).
- **CRDTs:** Built-in `GCounter`, `LWWRegister` for distributed state.
- **Mesh Networking:** P2P primitives (`Listen`, `Connect`, `Send`) via libp2p.

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

**File:** 
$file_path

$description

```$lang
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

**File:** 
$file_path

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

# GRAMMAR RULES
cat >> "$OUTPUT_FILE" << 'EOF'

## Grammar Rules

### Sentence Patterns

| Pattern | Example | Logic |
|---------|---------|-------|
| Universal | "All cats are mammals" | ∀x(Cat(x) → Mammal(x)) |
| Existential | "Some dogs bark" | ∃x(Dog(x) ∧ Bark(x)) |
| Generic | "Birds fly" | Gen x(Bird(x) → Fly(x)) |
| Imperative | "Let x be 5." | let x = 5; |
| Conditional | "If x > 0: Return x." | if x > 0 { return x; } |
| Network | "Send msg to peer." | peer.send(msg).await; |
| Concurrency | "Launch a task to fn." | tokio::spawn(fn()); |

### Quantifier Kinds

| Kind | Trigger | Symbol | Semantics |
|------|---------|--------|-----------|
| Universal | "all", "every", "each" | ∀ | True for every individual |
| Existential | "some", "a", "an" | ∃ | True for at least one |
| Generic | Bare plural ("birds") | Gen | Law-like/characteristic |
| Negative | "no", "none" | ¬∃ | True for none |

### Imperative Statements

| Statement | Syntax | Semantics |
|-----------|--------|-----------|
| Let | `Let [mut] x [: Type] be expr.` | Variable declaration |
| Set | `Set x to expr.` | Mutation |
| If | `If condition: ... Otherwise: ...` | Control flow |
| While | `While condition: ...` | Loop |
| Call | `Call f with x.` | Function invocation |
| Check | `Check that P.` | Mandatory security assertion |
| Assert | `Assert that P.` | Debug assertion |
| Give | `Give x to f.` | Ownership move |
| Show | `Show x to f.` | Immutable borrow |

EOF

# STATISTICS
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

# CRDT
CRDT_LINES=0
for f in logos_core/src/crdt/*.rs; do
    if [ -f "$f" ]; then
        CRDT_LINES=$((CRDT_LINES + $(wc -l < "$f")))
    fi
done
echo "CRDT (logos_core/src/crdt/):          $CRDT_LINES lines" >> "$OUTPUT_FILE"

# Network
NETWORK_LINES=0
for f in logos_core/src/network/*.rs; do
    if [ -f "$f" ]; then
        NETWORK_LINES=$((NETWORK_LINES + $(wc -l < "$f")))
    fi
done
echo "Network (logos_core/src/network/):    $NETWORK_LINES lines" >> "$OUTPUT_FILE"

# VFS
VFS_LINES=0
for f in logos_core/src/fs/*.rs; do
    if [ -f "$f" ]; then
        VFS_LINES=$((VFS_LINES + $(wc -l < "$f")))
    fi
done
echo "VFS (logos_core/src/fs/):             $VFS_LINES lines" >> "$OUTPUT_FILE"

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


# ==============================================================================
# LEXICON DATA
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Lexicon Data

The lexicon defines all vocabulary entries that drive the lexer and parser behavior.
EOF
add_file "assets/lexicon.json" "Lexicon JSON" "Core vocabulary definition."

# ==============================================================================
# LEXER & TOKENIZATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Lexer & Tokenization

Transforms text into classified tokens.
EOF

add_file "src/token.rs" "Token Definitions" "Token enum and types."
add_file "src/lexer.rs" "Lexer Implementation" "Tokenization logic and classification."

# ==============================================================================
# PARSER & AST
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Parser & AST

Recursive descent parser supporting both Logic and Imperative modes.
EOF

add_file "src/ast/mod.rs" "AST Module" "AST re-exports."
add_file "src/ast/logic.rs" "Logic AST" "Declarative expressions (Predicate, Quantifier)."
add_file "src/ast/stmt.rs" "Statement AST" "Imperative statements (Let, Set, If)."
add_file "src/parser/mod.rs" "Parser Core" "Parser struct and entry points."
add_file "src/parser/clause.rs" "Clause Parsing" "Sentence structure handling."
add_file "src/parser/quantifier.rs" "Quantifier Parsing" "Quantifier handling."
add_file "src/parser/verb.rs" "Verb Parsing" "Verb phrase handling."
add_file "src/parser/noun.rs" "Noun Parsing" "Noun phrase handling."

# ==============================================================================
# SEMANTICS
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Semantic Analysis

Lambda calculus and discourse representation.
EOF

add_file "src/lambda.rs" "Lambda Calculus" "Compositional semantics."
add_file "src/drs.rs" "DRS" "Discourse Representation Structures."
add_file "src/context.rs" "Context" "Discourse context."

# ==============================================================================
# TRANSPILATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Transpilation

Converting Logic AST to string representations.
EOF

add_file "src/transpile.rs" "Transpiler" "AST -> String conversion."
add_file "src/formatter.rs" "Formatter" "Output formatting."

# ==============================================================================
# CODE GENERATION (IMPERATIVE)
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Code Generation

Converting Imperative AST to Rust.
EOF

add_file "src/codegen.rs" "Codegen" "Stmt -> Rust conversion."
add_file "src/compile.rs" "Compiler" "Compilation pipeline."
add_file "src/scope.rs" "Scope" "Scope management."

# ==============================================================================
# TYPE ANALYSIS
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Type Analysis

Two-pass type checking and discovery.
EOF

add_file "src/analysis/mod.rs" "Analysis" "Module entry."
add_file "src/analysis/registry.rs" "Registry" "Type definitions."
add_file "src/analysis/discovery.rs" "Discovery" "First pass discovery."

# ==============================================================================
# RUNTIME
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Logos Core Runtime

Standard library for compiled programs.
EOF

add_file "logos_core/src/lib.rs" "Runtime Lib" "Core library."
add_file "logos_core/src/types.rs" "Types" "Runtime types."

# ==============================================================================
# WEB APPLICATION
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Web Application

Dioxus frontend components.
EOF

add_file "src/main.rs" "Main" "Entry point."
if [ -d "src/ui" ]; then
    add_file "src/ui/app.rs" "App" "Root component."
fi

# ==============================================================================
# PROBLEM GENERATOR
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Problem Generator

Curriculum and exercise generation.
EOF

add_file "src/content.rs" "Content" "Curriculum loader."
add_file "src/generator.rs" "Generator" "Problem generator."
add_file "src/grader.rs" "Grader" "Semantic grader."

# ==============================================================================
# TESTS
# ==============================================================================
cat >> "$OUTPUT_FILE" << 'EOF'
## Integration Tests
EOF

add_test_description "tests/phase1_garden_path.rs" "Phase 1: Garden Path" "Structural ambiguity tests." "The horse raced past the barn fell."
add_test_description "tests/phase21_block_headers.rs" "Phase 21: Imperative" "Imperative mode tests." "## Main"
add_test_description "tests/phase24_codegen.rs" "Phase 24: Codegen" "Code generation tests." "Let x be 5."

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

---

**Note:** This documentation is auto-generated. Run 
./generate-docs.sh to regenerate.
EOF

# ==============================================================================
# SUMMARY
# ==============================================================================
echo ""
echo "Documentation generated: $OUTPUT_FILE"
echo ""
echo "Summary:"
echo "--------"
echo "  Source files: $SRC_LINES"
echo "  Test files:   $TEST_LINES"
echo "  Total lines:  $TOTAL_LINES"
echo ""
echo "  Documentation size: $(du -h "$OUTPUT_FILE" | cut -f1)"
echo ""
echo "Done! View with: cat $OUTPUT_FILE"
chmod +x "$OUTPUT_FILE"