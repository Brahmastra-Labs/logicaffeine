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

### Statement AST
**File:** `src/ast/stmt.rs`

Imperative AST types: Stmt (Let, Set, If) and Expr (Literal, Call, BinaryOp).



---
### Compiler Driver
**File:** `src/compile.rs`

Orchestrates the compilation pipeline.



---
### Code Generator
**File:** `src/codegen.rs`

Emits Rust code from Stmt AST.



---
### Scope Management
**File:** `src/scope.rs`

Variable scoping and ownership tracking.



---
### Analysis Module
**File:** `src/analysis/mod.rs`

Entry point for semantic analysis.



---
### Type Registry
**File:** `src/analysis/registry.rs`

Stores type definitions (Structs, Enums).



---
### Discovery Pass
**File:** `src/analysis/discovery.rs`

First pass for type discovery.



---
### Ownership Analysis
**File:** `src/analysis/ownership.rs`

Linear types and ownership tracking.



---
### Verification Bridge
**File:** `src/verification.rs`

Mapping AST to Verification IR.



---
### Runtime Lib
**File:** `logos_core/src/lib.rs`

Core runtime types and traits.



---
## Relevant Tests
#### Phase 21: Blocks
**File:** `tests/phase21_block_headers.rs`
Block structure parsing.
---
#### Phase 23: Types
**File:** `tests/phase23_types.rs`
Type system tests.
---
#### Phase 24: Codegen
**File:** `tests/phase24_codegen.rs`
Rust code generation verification.
---
#### Phase 31: Structs
**File:** `tests/phase31_structs.rs`
User-defined structs.
---
#### Phase 54: Concurrency
**File:** `tests/phase54_concurrency.rs`
Go-like concurrency primitives.
---
#### Phase 50: Security
**File:** `tests/phase50_security.rs`
Policy enforcement.
---
