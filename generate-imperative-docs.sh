#!/bin/bash

# LOGOS IMPERATIVE - Imperative Language Documentation Generator
# Generates documentation for the LOGOS programming language (imperative mode).

OUTPUT_FILE="LOGOS_IMPERATIVE_DOCS.md"
echo "Generating IMPERATIVE documentation..."

# ==============================================================================
# HEADER & TOC
# ==============================================================================
cat > "$OUTPUT_FILE" << 'EOF'
# LOGOS - Imperative Language Documentation

## Overview
This document covers the **Imperative Layer** of the LOGOS system: the executable programming language, type system, and code generation.

## Table of Contents
1. [Architecture](#architecture)
2. [Imperative AST](#imperative-ast)
3. [Type Analysis](#type-analysis)
4. [Code Generation](#code-generation)
5. [Runtime & verification](#runtime--verification)
6. [Relevant Tests](#relevant-tests)

## Architecture

**Pipeline:**
`Input (Imperative Block) -> Parser (Stmt Mode) -> AST (Stmt) -> Type Analysis (Two-Pass) -> Verification (Z3) -> Codegen (Rust)`

**Key Components:**
*   **AST (`ast/stmt.rs`):** Imperative nodes: `Let`, `Set`, `Call`, `If`, `While`, `Return`, `Assert`, `Give`.
*   **Type Analysis (`analysis/`):** Two-pass compilation (`DiscoveryPass` -> `TypeRegistry`). Handles structs, enums, and generics.
*   **Codegen (`codegen.rs`):** Emits valid Rust code.
*   **Runtime (`logos_core/`):** Standard library (IO, Collections, Network, CRDT).
*   **Verification (`logos_verification/`):** SMT-based static analysis for `Assert` statements.

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
            echo "```"
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

# Imperative AST
add_file "src/ast/stmt.rs" "Statement AST" "Imperative AST types: Stmt (Let, Set, If) and Expr (Literal, Call, BinaryOp)."

# Compilation & Codegen
add_file "src/compile.rs" "Compiler Driver" "Orchestrates the compilation pipeline."
add_file "src/codegen.rs" "Code Generator" "Emits Rust code from Stmt AST."
add_file "src/scope.rs" "Scope Management" "Variable scoping and ownership tracking."

# Type Analysis
add_file "src/analysis/mod.rs" "Analysis Module" "Entry point for semantic analysis."
add_file "src/analysis/registry.rs" "Type Registry" "Stores type definitions (Structs, Enums)."
add_file "src/analysis/discovery.rs" "Discovery Pass" "First pass for type discovery."
add_file "src/analysis/ownership.rs" "Ownership Analysis" "Linear types and ownership tracking."

# Verification
add_file "src/verification.rs" "Verification Bridge" "Mapping AST to Verification IR."

# Runtime (Headers only)
add_file "logos_core/src/lib.rs" "Runtime Lib" "Core runtime types and traits."

# Tests
cat >> "$OUTPUT_FILE" << 'EOF'
## Relevant Tests
EOF

add_test_description "tests/phase21_block_headers.rs" "Phase 21: Blocks" "Block structure parsing."
add_test_description "tests/phase23_types.rs" "Phase 23: Types" "Type system tests."
add_test_description "tests/phase24_codegen.rs" "Phase 24: Codegen" "Rust code generation verification."
add_test_description "tests/phase31_structs.rs" "Phase 31: Structs" "User-defined structs."
add_test_description "tests/phase54_concurrency.rs" "Phase 54: Concurrency" "Go-like concurrency primitives."
add_test_description "tests/phase50_security.rs" "Phase 50: Security" "Policy enforcement."

echo "Done! View with: cat $OUTPUT_FILE"