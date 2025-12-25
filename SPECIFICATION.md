# LOGOS Language Specification

**Version:** 0.5.2-draft ("The Council Edition")
**Status:** Living Document

---

## Table of Contents

### 1. [Introduction & Philosophy](#1-introduction--philosophy)
- [1.1 Vision](#11-vision)
- [1.2 Core Principles](#12-core-principles)
- [1.3 Design Goals](#13-design-goals)
- [1.4 Comparison Matrix](#14-comparison-matrix)
- [1.5 Hello World](#15-hello-world)
- [1.6 A More Substantial Example](#16-a-more-substantial-example)

### 2. [Lexical Structure](#2-lexical-structure)
- [2.1 Source Format](#21-source-format)
- [2.2 Structural Elements](#22-structural-elements)
- [2.3 Module Definition](#23-module-definition)
  - [2.3.1 Context Headers](#231-context-headers)
  - [2.3.2 The Abstract Rule (Imports via Hyperlinks)](#232-the-abstract-rule-imports-via-hyperlinks)
  - [2.3.3 URI Schemes](#233-uri-schemes)
  - [2.3.4 Resource Embedding](#234-resource-embedding)
  - [2.3.5 Interface Implementation](#235-interface-implementation)
  - [2.3.6 Dependency Resolution & Lockfiles](#236-dependency-resolution--lockfiles)
  - [2.3.7 Two-Pass Compilation (Type Discovery)](#237-two-pass-compilation-type-discovery)
  - [2.3.8 Visibility Rules](#238-visibility-rules-the-open-conversation-rule)
  - [2.3.9 Intrinsic Injection (Compiler Bootstrap)](#239-intrinsic-injection-compiler-bootstrap)
- [2.4 Function Definition](#24-function-definition)
- [2.5 Block Scoping (The Narrative Rule)](#25-block-scoping-the-narrative-rule)
  - [2.5.1 Indentation Rules](#251-indentation-rules)
  - [2.5.2 Lexer Indentation Handling](#252-lexer-indentation-handling)
  - [2.5.3 Mode-Sensitive Token Consumption](#253-mode-sensitive-token-consumption)
- [2.6 Proof Blocks](#26-proof-blocks)
- [2.7 Code Fences (FFI)](#27-code-fences-ffi)
- [2.8 Comments and Documentation](#28-comments-and-documentation)
- [2.9 Reserved Words](#29-reserved-words)
  - [2.9.1 Identifier Syntax](#291-identifier-syntax)
  - [2.9.2 Multi-word Identifiers (Longest Match)](#292-multi-word-identifiers-longest-match)
  - [2.9.3 Greedy Declaration, Lazy Reference](#293-greedy-declaration-lazy-reference)
- [2.10 Mixed Math Syntax](#210-mixed-math-syntax)
- [2.11 Precedence and Grouping (The Monotonic Rule)](#211-precedence-and-grouping-the-monotonic-rule)

### 3. [Type System](#3-type-system)
- [3.1 Overview](#31-overview)
- [3.2 Base Types](#32-base-types)
- [3.3 Composite Types](#33-composite-types)
- [3.4 Dependent Types](#34-dependent-types)
- [3.5 Refinement Types](#35-refinement-types)
  - [3.5.1 Constraint Grammar](#351-constraint-grammar)
  - [3.5.2 Runtime Validation (The Inspection Rule)](#352-runtime-validation-the-inspection-rule)
  - [3.5.3 The Computability Restriction](#353-the-computability-restriction)
- [3.6 Generics (The Adjective System)](#36-generics-the-adjective-system)
  - [3.6.1 Type Expression Grammar](#361-type-expression-grammar-the-disambiguation-rule)
- [3.7 Type Inference](#37-type-inference)
- [3.8 Universe Hierarchy](#38-universe-hierarchy)

### 4. [English Grammar Specification](#4-english-grammar-specification)
- [4.1 Sentence Types](#41-sentence-types)
  - [4.1.1 The Canonical Phrasebook](#411-the-canonical-phrasebook)
  - [4.1.2 The Case Convention (Types vs. Predicates)](#412-the-case-convention-types-vs-predicates)
- [4.2 Declarative Sentences](#42-declarative-sentences)
- [4.3 Imperative Sentences](#43-imperative-sentences)
  - [4.3.1 Identifier Resolution Order](#431-identifier-resolution-order)
  - [4.3.2 Logical Assertions in Imperative Blocks](#432-logical-assertions-in-imperative-blocks)
- [4.4 Quantified Expressions](#44-quantified-expressions)
- [4.5 Conditional Expressions](#45-conditional-expressions)
- [4.6 Pattern Matching (The Inspection System)](#46-pattern-matching-the-inspection-system)
- [4.7 Ownership Verbs](#47-ownership-verbs)
- [4.8 Concurrency Phrases](#48-concurrency-phrases)

### 5. [Totality & Termination](#5-totality--termination)
- [5.1 The Halting Principle](#51-the-halting-principle)
- [5.2 Smart Inference](#52-smart-inference)
- [5.3 Explicit Decreasing Variants](#53-explicit-decreasing-variants)

### 6. [The Socratic Error System](#6-the-socratic-error-system)
- [6.1 Philosophy](#61-philosophy)
- [6.2 The Failure Type](#62-the-failure-type)
- [6.3 Example Error Output](#63-example-error-output)
- [6.4 Propagation](#64-propagation)

### 7. [Proof System](#7-proof-system)
- [7.1 Curry-Howard Correspondence](#71-curry-howard-correspondence)
- [7.2 Theorem Syntax](#72-theorem-syntax)
- [7.3 Proof Tactics](#73-proof-tactics)
- [7.4 Proof Obligations](#74-proof-obligations)
- [7.5 Proof Irrelevance](#75-proof-irrelevance)
- [7.6 Totality Checking](#76-totality-checking)
- [7.7 Manual Override (The Trust Verb)](#77-manual-override-the-trust-verb)

### 8. [Memory Model](#8-memory-model)
- [8.1 Ownership Semantics](#81-ownership-semantics)
- [8.2 Ownership Rules](#82-ownership-rules)
- [8.3 Examples](#83-examples)
- [8.4 Lifetime Inference](#84-lifetime-inference)
- [8.5 The Zone System (Manual Memory)](#85-the-zone-system-manual-memory)
  - [8.5.1 Nested Zone Containment](#851-nested-zone-containment)
- [8.6 Ownership and Concurrency](#86-ownership-and-concurrency)

### 9. [Concurrency Model](#9-concurrency-model)
- [9.1 Three-Layer Architecture](#91-three-layer-architecture)
- [9.2 Structured Concurrency (Core)](#92-structured-concurrency-core)
- [9.3 Proof Obligations for Concurrency](#93-proof-obligations-for-concurrency)
- [9.4 Channels and Pipelines (CSP)](#94-channels-and-pipelines-csp)
- [9.5 Agent Model (Distributed)](#95-agent-model-distributed)
- [9.6 Communication Verbs: Give vs Send](#96-communication-verbs-give-vs-send)
- [9.7 Agent Contracts](#97-agent-contracts)

### 10. [Standard Library](#10-standard-library)
- [10.1 Core Types](#101-core-types)
- [10.2 Numeric Tower](#102-numeric-tower)
- [10.3 Sequence Operations](#103-sequence-operations)
- [10.4 Mapping Operations](#104-mapping-operations)
- [10.5 IO Operations](#105-io-operations)
- [10.6 FFI Conventions](#106-ffi-conventions)
  - [10.6.1 Type Marshaling](#1061-type-marshaling)
  - [10.6.2 String Marshaling Safety](#1062-string-marshaling-safety)
- [10.7 Implementation Requirements](#107-implementation-requirements)

### 11. [Quality of Life](#11-quality-of-life)
- [11.1 String Interpolation](#111-string-interpolation)
- [11.2 Magic Slices and Ranges](#112-magic-slices-and-ranges)
- [11.3 The Socratic Compiler](#113-the-socratic-compiler)
- [11.4 Active Voice Enforcement](#114-active-voice-enforcement-style-linter)

### 12. [Compilation Pipeline](#12-compilation-pipeline)
- [12.1 Pipeline Overview](#121-pipeline-overview)
  - [12.1.1 Imperative Determinism](#1211-imperative-determinism-the-no-forest-rule)
- [12.2 Dual-Mode Compilation](#122-dual-mode-compilation)
- [12.3 Compilation Stages](#123-compilation-stages)
- [12.4 Error Messages (Socratic Style)](#124-error-messages-socratic-style)
- [12.5 Incremental Compilation](#125-incremental-compilation)
- [12.6 Source Mapping & Debug Information](#126-source-mapping--debug-information)

### 13. [The Live Codex (IDE)](#13-the-live-codex-ide)
- [13.1 Vision: Code as Conversation](#131-vision-code-as-conversation)
- [13.2 The Logic Visualizer](#132-the-logic-visualizer)
- [13.3 Real-time Proof Status](#133-real-time-proof-status)
- [13.4 The Teacher's Pass](#134-the-teachers-pass-style-enforcement)
- [13.5 Hot-Reloading Proofs](#135-hot-reloading-proofs)
- [13.6 Morphological Refactoring](#136-morphological-refactoring)

### 14. [Appendices](#14-appendices)
- [14.1 Grammar Summary (EBNF)](#141-grammar-summary-ebnf)
- [14.2 Type Inference Algorithm](#142-type-inference-algorithm)
- [14.3 Proof Checking Algorithm](#143-proof-checking-algorithm)
- [14.4 Comparison with Related Work](#144-comparison-with-related-work)
- [14.5 Future Extensions](#145-future-extensions)
- [14.6 Complete Examples](#146-complete-examples)

### 15. [Implementation Mechanics (The Engine Room)](#15-implementation-mechanics-the-engine-room)
- [15.0 The Dual-AST Architecture](#150-the-dual-ast-architecture)
  - [15.0.1 Type vs. Sort (Dual Classification)](#1501-type-vs-sort-dual-classification)
  - [15.0.2 The Parser Mode Switch](#1502-the-parser-mode-switch)
  - [15.0.3 Verb Parsing Trait Split](#1503-verb-parsing-trait-split)
- [15.1 The Adjective System (Generics)](#151-the-adjective-system-generics)
- [15.2 The Zone System (Memory Arenas)](#152-the-zone-system-memory-arenas)
- [15.3 The Socratic Error System (Traceability)](#153-the-socratic-error-system-traceability)
- [15.4 Agents & Wire Protocol](#154-agents--wire-protocol)
- [15.5 Totality Checking (Termination)](#155-totality-checking-termination)
- [15.6 Structured Concurrency](#156-structured-concurrency)
- [15.7 Mixed Math Parsing](#157-mixed-math-parsing)
- [15.8 Refinement Types (Constraint Solving)](#158-refinement-types-constraint-solving)

### 16. [Implementation Roadmap (v0.5.0 Transition)](#16-implementation-roadmap-v050-transition)
- [Phase 1: The Architectural Split](#phase-1-the-architectural-split)
- [Phase 2: Imperative Scope & Resolution](#phase-2-imperative-scope--resolution)
- [Phase 3: The Imperative AST](#phase-3-the-imperative-ast-stmt)
- [Phase 4: Codegen Backend](#phase-4-codegen-backend-rust-emission)
- [Phase 5: Verification Integration](#phase-5-verification-integration)

---

## 1. Introduction & Philosophy

### 1.1 Vision

LOGOS is a programming language where **English is the syntax**, **logic is the semantics**, **proofs are the types**, and **Rust is the runtime**.

The name comes from the Greek λόγος — meaning word, reason, and principle. In LOGOS, these three concepts unify: words become executable reason, and reason becomes verifiable principle.

### 1.2 Core Principles

| Principle | Description |
|-----------|-------------|
| **English IS Code** | The source text is readable English prose, not comments wrapped around symbols |
| **Curry-Howard** | Types are propositions; programs are proofs |
| **English Ownership** | Memory safety via natural verbs: *give*, *show*, *let modify* |
| **Kernel Architecture** | Declarative logic (Logicaffeine) is the kernel; imperative code wraps it via `Assert` |
| **Structured Verification** | Full proof-checking with structured concurrency for tractable reasoning |
| **Zero-Cost Compilation** | Transpile to Rust for LLVM-optimized native binaries |
| **Proof Irrelevance** | Proofs verify at compile-time but add zero runtime overhead |

### 1.3 Design Goals

1. **Correctness by Construction** — If it compiles, it is proven correct
2. **Performance without Sacrifice** — C-level speed through Rust codegen
3. **Accessibility without Compromise** — Natural language that compiles deterministically
4. **Scale without Chaos** — Local structured concurrency, distributed agents
5. **Delight without Complexity** — The language should feel like a conversation

### 1.4 Comparison Matrix

| Feature | LOGOS | Lean 4 | Rust | Python | Zig |
|---------|-------|--------|------|--------|-----|
| Primary Syntax | English | Lean DSL | Symbols | Symbols | C-like |
| Type System | Dependent + Refinement | Dependent | Affine + Generics | Dynamic | Comptime |
| Proof Obligations | Required | Required | Optional (unsafe) | None | Testing |
| Memory Model | Ownership (English) | GC | Ownership (symbols) | GC | Manual/Arenas |
| Performance | Native (via Rust) | Native | Native | Interpreted | Native |
| Concurrency | Structured + Agents | Tasks | async/channels | GIL-limited | Async |
| File Format | Markdown | .lean | .rs | .py | .zig |
| Generics Syntax | `Stack of [Things]` | `Stack α` | `Stack<T>` | Duck typing | `Stack(T)` |
| Manual Memory | Zone System | N/A | unsafe + arenas | N/A | Allocators |

### 1.5 Hello World

```markdown
# Hello World

To run:
    Show "Hello, World!" to the console.
```

This compiles to:

```rust
fn main() -> Result<(), Error> {
    println!("Hello, World!");
    Ok(())
}
```

### 1.6 A More Substantial Example

```markdown
# Factorial

A module demonstrating recursion with totality proof.

## Definition

The factorial of a natural number n is:
    If n is 0, then 1.
    Otherwise, n times the factorial of n minus 1.

> **Theorem:** The factorial function terminates for all natural numbers.
> *Proof:* By structural induction on n. The recursive call uses n - 1,
> which is strictly smaller than n for all n > 0. Auto.

## Main

To run:
    Let result be the factorial of 10.
    Show result to the console.
```

---

## 2. Lexical Structure

### 2.1 Source Format

LOGOS source files are **Markdown documents** with the `.md` extension. The compiler interprets Markdown structure as program structure.

### 2.2 Structural Elements

| Markdown Element | LOGOS Semantics |
|------------------|-----------------|
| `# Header` | Module or type definition |
| `## Subheader` | Function or section definition |
| `Indented text` | Block scope (following a colon `:`) |
| `> Blockquote` | Theorem, proof, or assertion |
| `` ``` `` Code fence | FFI escape to Rust/C |
| Plain paragraph | Documentation (ignored by compiler) |

### 2.3 Module Definition

Headers define module boundaries:

```markdown
# Math.Arithmetic

This module provides basic arithmetic operations.

## Addition

To add a to b:
    Return a plus b.
```

The `#` header creates module `Math.Arithmetic`. The `##` header creates function `addition` within that module.

#### 2.3.1 Context Headers

The parsing mode is determined by the section header:

| Header Pattern | Parsing Mode | Allowed Constructs |
|----------------|--------------|-------------------|
| `## Definition` / `## Types` | Declarative | Type definitions, records, predicates |
| `## Theorem` / `## Axioms` / `## Lemma` | Logic | Propositions, proofs, logical assertions |
| `## Main` / `## To [Verb]` | Imperative | Statements, control flow, mutations |

**Mode Restrictions:**
- Declarative Mode: Imperative verbs (`Return`, `Call`, `Set`) are forbidden
- Logic Mode: Mutations and side effects are forbidden
- Imperative Mode: All constructs allowed

**The `If` Disambiguation:**

The keyword `If` has different semantics depending on the parsing mode:

| Mode | `If` Semantics | Example |
|------|----------------|---------|
| Logic (`## Theorem`) | Logical implication (→) | `If x is human, x is mortal.` |
| Imperative (`## Main`) | Control flow (branch) | `If x equals 5: Return true.` |

To embed logical propositions in imperative blocks, use assertion wrappers:
- `Assert that if P, Q.` — compile-time proof obligation
- `Check that if P, Q.` — runtime validation

#### 2.3.2 The Abstract Rule (Imports via Hyperlinks)

The first paragraph following a module header is the **Abstract**. Any markdown links in the abstract are treated as dependency imports.

```markdown
# The Web Server

This module implements an HTTP server. It relies on the
[Standard Library](logos:std) and the [Async Runtime](https://github.com/logos/async/v1.2.md).

## To Start Server
    ...
```

**Semantics:**
- The compiler resolves each URI and loads the linked module
- The link text becomes the module alias for use in code
- Links may appear anywhere in the abstract paragraph

#### 2.3.3 URI Schemes

| Scheme | Meaning | Example |
|--------|---------|---------|
| `logos:` | Standard Library | `[Math](logos:math)` |
| `file:` | Local relative path | `[Utils](file:./utils.md)` |
| `https:` | Remote specification | `[JSON](https://logos.pkg/json/v2.md)` |
| `git:` | Git repository | `[Physics](git://github.com/org/physics)` |
| `ipfs:` | Content-addressable | `[Core](ipfs:QmHash...)` |

**Using imported modules:**
```markdown
To query the database:
    Give the query to the Postgres Driver.
```

The alias "Postgres Driver" refers to the module linked in the abstract.

**Qualified Type Access:**

To reference a type exported by an imported module, use the `from` preposition:

```markdown
# My Application

This module uses the [Standard Library](logos:std).

## Definition

Let items be a List from Standard Library.
Let cache be a Map from Text to Integer from Standard Library.
```

**Syntax:**
```
[Type] from [Module Alias]
```

This syntax aligns with the generic type syntax (`Stack of Integers`) and avoids conflicts with possessive field access (`user's name`).

**Disambiguation:**
If both a local type and an imported type share a name, the local definition takes precedence. Use the qualified form to access the imported type explicitly.

#### 2.3.4 Resource Embedding

Static assets (schemas, templates, configuration) use markdown image syntax:

```markdown
# Database Schema

We define the user structure based on the ![User Schema](file:./schemas/user.sql).

## Definition

A User is a record matching the User Schema.
```

The compiler loads the resource and binds it to the alias.

#### 2.3.5 Interface Implementation

To declare that a module implements an interface defined elsewhere:

```markdown
# The File Logger

This module implements the [Logger Interface](logos:io/Logger).

## To Log Message
    ...
```

The compiler verifies all theorems and definitions from the linked interface are fulfilled.

**Interface Proof Obligations:**

If an interface defines theorems, the implementing module inherits them as **proof obligations** (goals).

**Interface Definition (Logger Interface):**
```markdown
# The Logger Interface

## To Log Message
    Abstract.

> **Theorem monotonic_count:** Each call to Log Message increases the log count.
```

**Implementation Requirement:**

The implementing module must provide a `Proof` block for each theorem:

```markdown
# The File Logger

This module implements the [Logger Interface](logos:io/Logger).

## To Log Message
    Append message to log_file.
    Increase log_count by 1.

> **Proof of monotonic_count:**
> The implementation increments log_count by 1 on each call.
> Since 1 > 0, the count strictly increases. Auto.
```

**Enforcement:**

| Obligation | Compile Result |
|------------|----------------|
| All proofs provided | Success |
| Missing proof | Error: "Module does not satisfy interface theorem 'X'" |
| Invalid proof | Error: "Proof of 'X' is incomplete or incorrect" |

This mechanism ensures that interface contracts are not just signatures but verifiable guarantees.

#### 2.3.6 Dependency Resolution & Lockfiles

When the compiler resolves imports, it generates a lockfile recording the exact content:

```toml
# logos.lock - Auto-generated, do not edit

[[dependency]]
alias = "Async Runtime"
uri = "https://github.com/logos/async/v1.2.md"
sha256 = "a1b2c3d4e5f6..."
resolved = "2025-12-24T10:30:00Z"

[[dependency]]
alias = "Standard Library"
uri = "logos:std"
version = "0.4.0"
```

**Resolution Rules:**

1. **First build:** Fetch all URIs, compute SHA-256 hashes, write `logos.lock`
2. **Subsequent builds:** If URI matches lockfile, verify hash; if mismatch, error
3. **Update:** `logos update` refreshes all dependencies and rewrites lockfile

**Integrity Verification:**

| URI Scheme | Verification Method |
|------------|---------------------|
| `https:` | SHA-256 of response body |
| `git:` | Commit hash |
| `ipfs:` | Content hash (inherent) |
| `file:` | SHA-256 of file contents |
| `logos:` | Bundled version number |

**Intrinsic Core (Implicit Prelude):**

The `logos:core` module is intrinsic—built directly into the compiler binary. It provides the foundation types (`Bool`, `Nat`, `Int`, `Text`, `Unit`) required to bootstrap parsing and dependency resolution.

**Unlike `logos:std`, `logos:core` is implicitly imported in every LOGOS file** (similar to Rust's prelude). Users do not need to link it explicitly.

| Module | Resolution | Import Style | Contents |
|--------|------------|--------------|----------|
| `logos:core` | Intrinsic (compiler-built-in) | **Implicit** (always available) | Bool, Nat, Int, Text, Unit |
| `logos:std` | Lockfile dependency | **Explicit** (requires link in Abstract) | Seq, Map, IO, Text ops |

**What `logos:core` Provides:**

- Primitive types: `Bool`, `Nat`, `Int`, `Real`, `Char`, `Text`, `Unit`
- Core traits: `Portable`, `Copy`
- Operators: `plus`, `minus`, `times`, etc.
- Keywords: `true`, `false`, `nothing`

**Example:**

```markdown
# My Module

This module uses the [Standard Library](logos:std).  ← Explicit import

## Main

Let x be 5.           ← 'Int' from logos:core (implicit)
Let y be true.        ← 'Bool' from logos:core (implicit)
Show x to the console. ← 'Show' from logos:std (explicit)
```

**Transitive Dependencies:**

If module A imports B, and B imports C, the lockfile records all three. Circular imports are forbidden; the compiler errors with the cycle path.

**The Separation Principle:**

To prevent circular dependencies and parsing conflicts, `logos:core` and `lexicon.json` serve distinct, non-overlapping roles:

| Source | Responsibility | Examples | May NOT Contain |
|--------|----------------|----------|-----------------|
| `logos:core` (TypeRegistry) | Type definitions, traits | `Int`, `Text`, `Bool`, `Copy`, `Portable` | Syntax keywords |
| `lexicon.json` (Lexer) | Syntax keywords, verb inflections | `all`, `some`, `Let`, `Set`, verb forms | Type definitions |

**Enforcement:** The compiler rejects any `lexicon.json` entry that collides with a `logos:core` type name.

#### 2.3.7 Two-Pass Compilation (Type Discovery)

To resolve syntactic ambiguities (e.g., `Stack of Integers` vs `Owner of House`), the parser must know whether a word is a Generic Type before parsing the expression. This requires **two-pass compilation** per module.

**Pass 1: Discovery**

Scans the module for:
- `# Header` declarations (module/type names)
- `## Definition` blocks (struct/enum definitions)
- `## Types` sections
- Generic type parameters (`[Things]` syntax)

**Outputs:**
- `TypeRegistry`: Maps type names to their definitions
- `GlobalSymbolTable`: Maps function names to signatures
- `GenericSet`: Set of types that are generic

**Pass 2: Body Parsing**

Parses function bodies, theorems, and logic using the Registry to resolve ambiguities:

| Expression | Registry Lookup | Interpretation |
|------------|-----------------|----------------|
| `Stack of Integers` | `Stack` in GenericSet | Generic instantiation |
| `Owner of House` | `Owner` not in GenericSet | Possessive/field access |

**Forward Reference Rule:**

Types may be used before their definition *within the same module* because Pass 1 discovers all definitions before Pass 2 parses bodies. However, **circular type definitions** (A contains B, B contains A) require explicit indirection (e.g., `Option` wrapper).

**Import Resolution:**

Imported modules are fully compiled (both passes) before the importing module begins Pass 1. This ensures all external types are available during Discovery.

**Two-Pass Algorithm (Mandatory for v0.5.0):**

A single-pass parser cannot disambiguate `Stack of Integers` (generic instantiation) from `Owner of House` (possessive) without knowing which nouns are generic types. The two-pass architecture is **required**, not optional.

**Pass 1: Discovery (Pseudocode)**
```
function discover(module):
    type_registry = {}
    global_symbols = {}
    generic_set = {}

    for each header in module:
        if header matches "# [Name]":
            register_module_name(Name)

        if header matches "# A [Name] of [Params]":
            generic_set[Name] = extract_params(Params)
            type_registry[Name] = GenericType(params)

        if header matches "## Definition":
            for each struct_def in block:
                type_registry[struct_name] = parse_struct_signature(struct_def)

        if header matches "## To [Verb]":
            signature = extract_function_signature(block)
            global_symbols[Verb] = signature

    return (type_registry, global_symbols, generic_set)
```

**Pass 2: Body Parsing (Pseudocode)**
```
function parse_bodies(module, registry):
    for each function_block in module:
        push_scope()

        for each argument in function.signature:
            bind_local(argument.name, argument.type)

        for each statement in function.body:
            if statement is "X of Y":
                if X in registry.generic_set:
                    parse_as_generic_instantiation(X, Y)
                else:
                    parse_as_possessive(X, Y)

            if statement is "Let [name] be [expr]":
                bind_local(name, infer_type(expr))

        pop_scope()
```

**Critical Invariant:** Pass 1 must complete for the **entire module** before Pass 2 begins. No lazy evaluation.

#### 2.3.8 Visibility Rules (The "Open Conversation" Rule)

For v0.1, LOGOS adopts simple visibility semantics:

| Entity | Default Visibility |
|--------|-------------------|
| Types (Records, Enums) | Public to importers |
| Functions | Public to importers |
| Record Fields | **Private** (encapsulation) |
| Module-level Variables | Public to importers |

**Field Access:**

To expose a field externally, mark it with the `public` adjective:

```markdown
A User has:
    a public name (Text).
    a password (Text).
```

In this example, `name` is accessible from importing modules, while `password` remains private to the defining module.

**Rationale:**

Types and functions are designed to be shared; that is the purpose of a module. However, fields represent internal state. Exposing them violates encapsulation and couples consumers to implementation details.

#### 2.3.9 Intrinsic Injection (Compiler Bootstrap)

The `logos:core` primitives (`Bool`, `Nat`, `Int`, `Text`, `Unit`) must be available before any user code parses. This creates a bootstrapping challenge: the import mechanism relies on types that haven't been defined yet.

**The Bootstrap Sequence:**

```
1. Compiler starts
2. Intrinsic Injection: Primitives hardcoded into TypeRegistry
3. logos:core module constructed in-memory (not parsed from .md)
4. User file parsing begins
5. Imports resolved using populated TypeRegistry
```

**Intrinsic Types (Pre-populated):**

| Type | Kind | Rust Equivalent |
|------|------|-----------------|
| `Bool` | Primitive | `bool` |
| `Nat` | Primitive | `u64` |
| `Int` | Primitive | `i64` |
| `Real` | Primitive | `f64` |
| `Char` | Primitive | `char` |
| `Text` | Composite | `String` |
| `Unit` | Primitive | `()` |

**Intrinsic Traits:**

| Trait | Purpose |
|-------|---------|
| `Portable` | Type can cross agent boundaries |
| `Copy` | Type has value semantics |

**Intrinsic Functions:**

| Function | Signature |
|----------|-----------|
| `plus` | `(a: Nat, b: Nat) → Nat` (and overloads) |
| `minus` | `(a: Int, b: Int) → Int` (and overloads) |
| `times` | `(a: Nat, b: Nat) → Nat` (and overloads) |
| `equals` | `(a: T, b: T) → Bool` (generic) |

**Implementation:**

```rust
fn initialize_type_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::new();

    // Inject primitives (not parsed from markdown)
    registry.insert("Bool", TypeDef::intrinsic_bool());
    registry.insert("Nat", TypeDef::intrinsic_nat());
    registry.insert("Int", TypeDef::intrinsic_int());
    registry.insert("Real", TypeDef::intrinsic_real());
    registry.insert("Char", TypeDef::intrinsic_char());
    registry.insert("Text", TypeDef::intrinsic_text());
    registry.insert("Unit", TypeDef::intrinsic_unit());

    registry
}
```

**Why This Matters:**

Without Intrinsic Injection:
- The parser cannot recognize `(n: Nat)` as a type annotation
- Generic syntax `Stack of [Things]` cannot distinguish types from nouns
- The `logos:std` import would fail (needs `Text` for URI parsing)

The intrinsic types are the "primordial soup" from which all other types derive.

### 2.4 Function Definition

Functions are introduced with "To [verb] [parameters]:" followed by an indented block:

```markdown
## Computing the Maximum

To find the maximum of a and b:
    If a is greater than b:
        Return a.
    Otherwise:
        Return b.
```

**Ownership in Parameters:**

Function parameters define ownership requirements using adjectives:

| Pattern | Meaning | Rust |
|---------|---------|------|
| `To process (data: Text):` | Takes ownership (move) | `fn process(data: String)` |
| `To process (borrowed data: Text):` | Immutable reference | `fn process(data: &String)` |
| `To process (mutable data: Text):` | Mutable reference | `fn process(data: &mut String)` |

The ownership adjective must match the caller's verb. If a function expects `borrowed data`, callers must use `Show`. If it expects `mutable data`, callers must use `Let modify`.

### 2.5 Block Scoping (The Narrative Rule)

Statements are written as plain English sentences ending with a period (`.`). A colon (`:`) at the end of a line opens an indented block.

```markdown
Let x be 5.
Set y to x plus 3.
Return y.
```

**Control flow with blocks:**

```markdown
If the list is empty:
    Return nothing.
Otherwise:
    Let first be the head of the list.
    Return first.
```

#### 2.5.1 Indentation Rules

Scope is strictly defined by indentation relative to the parent statement.

**Rules:**
1. A colon (`:`) at the end of a line opens a new block
2. The block continues while subsequent lines are indented deeper than the parent
3. The block terminates when indentation returns to the parent level or shallower
4. Each statement ends with a period (`.`)

**Example with scope annotations:**
```markdown
If x is greater than 0:                  ← Opens block
    Log "Positive".                      ← Inside block
    Return true.                         ← Inside block

Log "Continuing...".                     ← Outside block (runs after If)
```

Indentation uses 4 spaces per level. Tabs are forbidden.

#### 2.5.2 Lexer Indentation Handling

The lexer generates synthetic tokens to represent block structure:

| Token | Generated When |
|-------|----------------|
| `INDENT` | Indentation increases from previous non-empty line |
| `DEDENT` | Indentation decreases from previous non-empty line |
| `NEWLINE` | Line ends (not inside a block continuation) |

**Algorithm:**

1. Maintain an indentation stack (initially `[0]`)
2. For each non-empty line:
   - If indentation > stack top: push new level, emit `INDENT`
   - If indentation < stack top: pop levels, emit `DEDENT` for each pop
   - If indentation == stack top: emit `NEWLINE`
3. At end of input: emit `DEDENT` for each remaining level > 0

**Example Token Stream:**
```
Input:
    If x > 0:
        Log "Positive".
        Return true.
    Log "Done".

Tokens:
    IF, IDENTIFIER(x), GT, NUMBER(0), COLON, NEWLINE
    INDENT
    LOG, STRING("Positive"), PERIOD, NEWLINE
    RETURN, TRUE, PERIOD, NEWLINE
    DEDENT
    LOG, STRING("Done"), PERIOD, NEWLINE
```

This design follows Python's lexer approach, making block structure explicit at the token level.

**Implementation Requirement:**

The Lexer is **stateful**. It maintains an indentation stack across the entire input. This is a departure from the current Logicaffeine lexer (which is stateless per-token). The Lexer must track:

1. The current indentation depth stack
2. Whether the previous non-empty line ended with a colon (block opener)

This state persists throughout the file; the lexer cannot be restarted mid-document without losing indentation context.

**Mandatory Two-Pass Lexing Architecture:**

Mixing whitespace-sensitive logic into the existing word classifier results in unmaintainable code. The lexer MUST use a two-stage pipeline:

1. **`LineLexer` (Pass 1):** Reads raw text. Handles **only** indentation, comments, and line breaks. Emits a stream of lines with metadata (indent level). Inserts `INDENT`/`DEDENT` tokens.
2. **`WordLexer` (Pass 2):** Consumes the content *between* indentation tokens to classify words (`Let`, `Set`, `Identifier`).

This decouples block scoping from word classification, allowing the existing lexer logic to remain largely intact.

**Architecture Diagram:**

```
Raw Input → LineLexer → WordLexer → Token Stream
              ↓
         INDENT/DEDENT
```

**Stage 1 (LineLexer):**
1. Read raw lines from input
2. Calculate indentation delta from previous line
3. Emit `INDENT` or `DEDENT` tokens as needed
4. Pass remaining line content to Stage 2

**Stage 2 (WordLexer):**
1. Split line content by whitespace and punctuation
2. Classify words using existing `lexicon.json` lookup
3. Emit content tokens (`NOUN`, `VERB`, `IDENTIFIER`, etc.)

**Token Type Extension:**

The `TokenType` enum must include:

```rust
pub enum TokenType {
    // Block structure
    Indent,
    Dedent,
    Newline,

    // ... existing variants
}
```

**Interface Signatures:**

```rust
/// Stage 1: Line-level lexer handling indentation
pub struct LineLexer<'a> {
    input: &'a str,
    lines: std::str::Lines<'a>,
    indent_stack: Vec<usize>,
    pending_dedents: usize,
}

pub struct Line<'a> {
    pub content: &'a str,
    pub indent_level: usize,
}

pub enum LineToken<'a> {
    Indent,
    Dedent,
    Newline,
    Content(Line<'a>),
    Eof,
}

impl<'a> LineLexer<'a> {
    pub fn new(input: &'a str) -> Self;
    pub fn next_token(&mut self) -> LineToken<'a>;
}

/// Stage 2: Word-level lexer (wraps existing Lexer logic)
pub struct WordLexer<'a, 'int> {
    line_lexer: LineLexer<'a>,
    current_line: Option<Line<'a>>,
    word_pos: usize,
    interner: &'int mut Interner,
    lexicon: &'a Lexicon,
}

impl<'a, 'int> WordLexer<'a, 'int> {
    pub fn new(input: &'a str, interner: &'int mut Interner, lexicon: &'a Lexicon) -> Self;
    pub fn next_token(&mut self) -> Token;
}
```

#### 2.5.3 Mode-Sensitive Token Consumption

The **Lexer** generates `INDENT`/`DEDENT` tokens unconditionally for all modes. The **Parser** determines whether to consume or ignore them based on the current parsing mode.

| Parsing Mode | INDENT/DEDENT Handling |
|--------------|------------------------|
| Declarative (`## Theorem`, `## Logic`) | Ignored (treated as whitespace) |
| Imperative (`## Main`, `## To [Verb]`) | Structural (defines block scope) |

**Rationale:**

In Logicaffeine-style Logic Mode, proofs and theorems use natural paragraph flow. Whitespace is insignificant. In LOGOS Imperative Mode, indentation defines program structure (like Python).

**Implementation:**

The Lexer remains **stateless regarding parsing mode**. It always emits `INDENT`/`DEDENT` tokens. The Parser's `consume_token()` method checks the current mode:

```rust
fn consume_token(&mut self) -> Token {
    let token = self.lexer.next();
    match self.mode {
        ParserMode::Declarative => {
            // Skip indentation tokens in logic mode
            if matches!(token.kind, TokenType::Indent | TokenType::Dedent) {
                return self.consume_token(); // Recursive skip
            }
        }
        ParserMode::Imperative => {
            // Use indentation tokens for block structure
        }
    }
    token
}
```

This decoupling allows the same Lexer to serve both Logicaffeine compatibility and LOGOS systems programming.

### 2.6 Proof Blocks

Blockquotes (`>`) introduce theorems, lemmas, and proofs:

```markdown
> **Theorem:** For all natural numbers n, n plus 0 equals n.
>
> *Proof:* By induction on n.
> - **Base:** When n is 0, 0 plus 0 equals 0 by definition.
> - **Step:** Assume n plus 0 equals n. Then (n + 1) plus 0 equals
>   (n plus 0) plus 1, which equals n plus 1 by hypothesis.
```

### 2.7 Code Fences (FFI)

Triple backticks introduce raw Rust or C code:

````markdown
```rust
fn fast_multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b)
}
```

To multiply a by b quickly:
    Call the Rust function fast_multiply with a and b.
````

### 2.8 Comments and Documentation

Plain paragraphs are documentation and are ignored by the compiler:

```markdown
# Sorting

This module implements various sorting algorithms. The quicksort
implementation below uses the Hoare partition scheme for efficiency.

## Quicksort

To sort a list:
    ...
```

### 2.9 Reserved Words

The following English words have special meaning in LOGOS:

**Control Flow:**
`if`, `then`, `otherwise`, `return`, `repeat`, `while`, `for`, `every`, `until`, `inspect`, `consider`, `decreasing`, `try`

**Declarations:**
`let`, `set`, `to`, `be`, `is`, `are`, `has`, `have`

**Ownership:**
`give`, `show`, `modify`, `borrow`, `own`, `copy`

**Memory:**
`zone`, `inside`, `allocate`

**Quantifiers:**
`all`, `every`, `some`, `any`, `no`, `none`, `most`, `few`, `many`

**Logic:**
`and`, `or`, `not`, `implies`, `if and only if`, `therefore`, `because`

**Proofs:**
`theorem`, `lemma`, `proof`, `assume`, `suppose`, `hence`, `thus`, `qed`, `auto`, `trust`, `because`

**Concurrency:**
`attempt`, `await`, `simultaneously`, `concurrently`, `agent`, `stream`, `pour`, `spawn`, `send`, `reply`

**Types:**
`a`, `an`, `the`, `of`, `where`, `such that`, `with`

#### 2.9.1 Identifier Syntax

Identifiers follow `snake_case` or `camelCase` conventions. The parser treats underscores within word boundaries as literal characters, not Markdown formatting markers.

**Valid identifiers:** `login_count`, `isValid`, `user_name`, `maxRetryAttempts`

Backticks are only required when an identifier would otherwise be ambiguous with reserved words or operators.

#### 2.9.2 Multi-word Identifiers (Longest Match)

LOGOS permits multi-word identifiers written as natural English phrases. The parser resolves ambiguity using the **Longest Match Rule**.

**The Rule:**
When the lexer encounters a token, it checks if extending the match with subsequent tokens forms a defined symbol. If so, it consumes all tokens as a single identifier.

**Example:**
```markdown
A user count is a Nat.

To increment the user count:
    Set user count to user count plus 1.
```

Here, `user count` is defined as a single identifier. The lexer consumes both tokens together wherever they appear in sequence.

**Resolution Priority:**
1. Check if the current token sequence matches a defined symbol
2. If multiple matches exist, prefer the longest
3. Fall back to single-token interpretation

**Implementation Note (Critical Distinction):**

The **Parser** resolves variable identifiers, NOT the Lexer. The Lexer produces individual word tokens. When the Parser expects an identifier (e.g., after `Let` or in an expression), it eagerly consumes adjacent `NOUN` tokens to form the longest known identifier in the current scope.

The static MWE pipeline (for keywords like `if and only if`) remains a post-lexing phase, but **variable names are constructed at parse-time**.

```
Token Stream: [LET] [user] [count] [BE] [5] [PERIOD]
                    └────┬────┘
                    Parser consumes as single identifier
                    (if "user count" is in scope)
```

**Dynamic Identifier Registration:**

In Imperative Mode, when a `Let` statement defines a multi-word identifier, the Parser registers it in the current `ScopeStack` immediately. Subsequent identifier lookups within the same scope will match the full phrase.

| Mode | MWE Pipeline |
|------|--------------|
| Logic (Declarative) | Static: Pre-built Trie from lexicon.json |
| Imperative | Dynamic: Parser updates Trie on `Let` definitions |

**Implementation Strategy:**

The Parser maintains a `ScopeStack` that includes a local `MweTrie`. When parsing `Let user count be 5`:

1. Parse `user count` as an unresolved noun phrase
2. Register `user count` in the current scope's symbol table AND local MweTrie
3. Future tokens `user count` resolve to the single identifier

This requires the Lexer to pass raw tokens to the Parser, which performs MWE collapse contextually.

**Static vs Dynamic MWE Resolution:**

| MWE Type | Resolution Phase | Example |
|----------|------------------|---------|
| Keywords/Operators | Post-Tokenization (static) | `if and only if`, `at least` |
| User-defined variables | During Parsing (dynamic) | `Let user count be 5` |

The existing `apply_mwe_pipeline` function should ONLY be used for static keywords and operators defined in `lexicon.json`. User-defined multi-word identifiers are resolved by the Parser's scope-aware lookup.

**Ambiguity Prevention:**
If `user` and `user count` are both defined, the parser prefers `user count` when followed by `count`. To force the shorter match, use explicit grouping:
```markdown
Let (user) count be 5.   ← Forces 'user' as separate identifier
```

**Argument Separation (The Comma Rule):**

Function arguments are separated by commas. This provides unambiguous parsing without complex lookahead.

**Rule:**
- Single argument: `Show user count.`
- Multiple arguments: `Show user, count.`
- Commas explicitly delimit argument boundaries

**Example:**
```markdown
Let user be "Alice".
Let count be 5.
Let user count be 10.

Show user count.      ← One argument: the variable 'user count' (value 10)
Show user, count.     ← Two arguments: 'user' ("Alice") and 'count' (5)
```

**Interaction with Longest Match:**

Longest Match applies **within** each comma-separated argument, not across them.

| Expression | Arguments | Interpretation |
|------------|-----------|----------------|
| `Show user count.` | 1 | `user count` as single identifier |
| `Show user, count.` | 2 | `user` and `count` separately |
| `Show user count, total.` | 2 | `user count` and `total` |

**Variadic Functions:**
Functions accepting variable arguments use commas consistently:
```markdown
Print "Hello", user, count.   ← Three arguments
```

**Parser Implementation (stop_at_comma):**

The `parse_noun_phrase` function in Imperative Mode must accept a `stop_at_comma: bool` parameter.

| Context | stop_at_comma | Behavior |
|---------|---------------|----------|
| Function argument list | `true` | Comma terminates noun phrase |
| Subject position | `false` | Consume full phrase |
| Object of preposition | `false` | Consume full phrase |

```rust
fn parse_noun_phrase(&mut self, stop_at_comma: bool) -> ParseResult<NounPhrase>;
```

When `stop_at_comma` is true and a comma is encountered, the parser immediately returns the current noun phrase without consuming the comma. The calling function handles argument list parsing.

This rule ensures deterministic parsing without requiring type information during lexing.

**Reserved Operator Words in Identifiers:**

Multi-word identifiers CANNOT contain reserved operator words. This prevents ambiguity between identifier boundaries and arithmetic expressions.

**Forbidden patterns:**
```markdown
Let gross profit plus tax be 100.     ← ERROR: 'plus' is an operator
Let total times factor be 50.          ← ERROR: 'times' is an operator
```

**Reserved operator words:**
`plus`, `minus`, `times`, `divided by`, `modulo`, `squared`, `cubed`, `to the power of`

**Valid patterns:**
```markdown
Let gross profit be 100.               ← OK: no operators
Let total factor be 50.                ← OK: no operators
Let profit plus tax be 100.            ← ERROR: cannot include 'plus'
```

The Longest Match rule applies ONLY to non-operator words. When the parser encounters an operator word, it terminates the current identifier and treats the operator as a binary expression.

#### 2.9.3 The Greedy Declaration, Lazy Reference Rule

Multi-word identifier resolution follows different strategies for declaration versus reference contexts.

**Declaration Context (Greedy):**

When parsing `Let [identifier] be [value]`, the parser consumes as many non-keyword tokens as possible to form the identifier:

```markdown
Let user count be 10.     → Declares identifier "user count"
Let total items sold be 5. → Declares identifier "total items sold"
```

**Reference Context (Lazy - Longest Established):**

When referencing identifiers in expressions, the parser prefers the **longest established symbol** in the current scope:

| Defined Symbols | Input | Resolution |
|-----------------|-------|------------|
| `user`, `count` | `user count` | Two variables: `user`, `count` |
| `user`, `user count` | `user count` | One variable: `user count` |
| `user count`, `total` | `user count total` | Two: `user count`, `total` |

**The Suffix Ban Rule:**

To prevent ambiguity, a new identifier **cannot** be a suffix of an existing identifier in the same scope:

```markdown
Let user count be 10.
Let count be 5.        ← ERROR: "count" is a suffix of "user count"
```

**Compiler Error:**
```
Cannot define 'count' — it conflicts with existing identifier 'user count'.
Suffix identifiers create ambiguous references.
```

**Rationale:**

Without this rule, `Set count to 20.` would be ambiguous: does it modify the standalone `count` or the `count` portion of `user count`? The Suffix Ban eliminates this class of ambiguity at definition time.

**Allowed Patterns:**

| First Definition | Second Definition | Status |
|------------------|-------------------|--------|
| `user count` | `total count` | OK (no suffix relationship) |
| `count` | `user count` | OK (count is prefix, not suffix) |
| `user count` | `count` | ERROR (suffix) |
| `user count` | `user` | ERROR (suffix) |

### 2.10 Mixed Math Syntax

LOGOS allows both prose and symbolic math, unified into the same AST:

**Prose form:**
```markdown
Let area be width times height.
Let hypotenuse be the square root of a squared plus b squared.
```

**Symbolic form (in backticks):**
```markdown
Let y be `mx + b`.
Let distance be `√(x² + y²)`.
```

**Mixed form:**
```markdown
Let discriminant be `b² - 4ac`.
If discriminant is negative, return no real solutions.
```

Both forms parse to identical AST nodes. The symbolic form supports:
- Standard operators: `+`, `-`, `*`, `/`, `^`
- Greek letters: `α`, `β`, `γ`, `θ`, `π`
- Special symbols: `√`, `∞`, `∑`, `∏`
- Subscripts/superscripts: `x₁`, `x²`

### 2.11 Precedence and Grouping (The Monotonic Rule)

To ensure "English is Code" remains readable and deterministic, LOGOS enforces strict rules on mathematical operators in prose.

**The Monotonic Rule:**
A single English prose clause may not mix different classes of operators (additive and multiplicative) without explicit grouping. Violating this rule produces a compile error.

**Operator Classes:**

| Class | Operators |
|-------|-----------|
| Additive | `plus`, `minus` |
| Multiplicative | `times`, `divided by`, `modulo` |
| Exponential | `to the power of`, `squared`, `cubed` |

**Forbidden (Compile Error):**
```markdown
Set x to a plus b times c.
```

This is rejected because `plus` (additive) and `times` (multiplicative) appear in the same clause without grouping.

**Allowed (Explicit Grouping):**
```markdown
Set x to `a + b * c`.           ← Backticks: standard math precedence (PEMDAS)
Set x to (a plus b) times c.    ← Parentheses: explicit grouping
Set x to a plus `b * c`.        ← Mixed: backticks group the subexpression
```

**Allowed (Monotonic):**
```markdown
Set x to a plus b plus c.       ← OK: same operator class (additive)
Set x to a times b times c.     ← OK: same operator class (multiplicative)
```

**Rationale:**
In English, "a plus b times c" is cognitively ambiguous. Does it mean "(a + b) × c" or "a + (b × c)"? Rather than impose an invisible precedence rule that surprises users, LOGOS refuses to guess. Explicit grouping prevents silent logic errors.

**Backtick Precedence:**
Inside backticks, standard mathematical precedence (PEMDAS) applies:
- Parentheses first
- Exponents
- Multiplication and Division (left to right)
- Addition and Subtraction (left to right)

---

## 3. Type System

### 3.1 Overview

LOGOS uses a **dependent type system** with **refinement types**, unified under the Curry-Howard correspondence. Every type is a proposition; every term is a proof.

### 3.2 Base Types

| English | Type | Rust Equivalent |
|---------|------|-----------------|
| a truth value | `Bool` | `bool` |
| a natural number | `Nat` | `u64` |
| an integer | `Int` | `i64` |
| a rational number | `Rat` | Custom |
| a real number | `Real` | `f64` |
| a character | `Char` | `char` |
| text | `Text` | `String` |
| nothing | `Unit` | `()` |

### 3.3 Composite Types

**Sequences:**
```markdown
a sequence of Integers          → Vec<Int>
a list of Users                  → Vec<User>
```

**Mappings:**
```markdown
a mapping from Text to Integers  → HashMap<Text, Int>
a dictionary of Names to Ages    → HashMap<Text, Nat>
```

**Optionals:**
```markdown
an optional Integer              → Option<Int>
possibly a User                  → Option<User>
```

**Results:**
```markdown
either an Integer or an Error    → Result<Int, Error>
a User or a Failure              → Result<User, Failure>
```

**Tuples:**
```markdown
a pair of Integer and Text       → (Int, Text)
a triple of X, Y, and Z          → (X, Y, Z)
```

**Records (Structs):**
```markdown
# A User

A User has:
    a name, which is Text.
    an age, which is a Nat.
    an email, which is Text.
```

Compiles to:
```rust
struct User {
    name: String,
    age: u64,
    email: String,
}
```

**Sum Types (Enumerations):**

Sum types define values that can be one of several variants. Each variant may optionally carry data.

```markdown
# A Shape

A Shape is either:
    A Circle with a radius (Real).
    A Rectangle with a width (Real) and a height (Real).
    A Point.
```

Compiles to:
```rust
enum Shape {
    Circle { radius: f64 },
    Rectangle { width: f64, height: f64 },
    Point,
}
```

**Matching Sum Types:**

Use the Inspection System (§4.6) to match on variants:

```markdown
Inspect the shape:
    If it is a Circle (radius):
        Return `π * radius²`.
    If it is a Rectangle (width, height):
        Return width times height.
    If it is a Point:
        Return 0.
```

**Field Binding Semantics:**

Variant fields are matched by **name**, not position. The binding variables in an inspection pattern must correspond to the field names defined in the sum type.

| Definition | Valid Pattern | Invalid Pattern |
|------------|---------------|-----------------|
| `Circle with a radius` | `Circle (radius)` | `Circle (r)` |
| `Rectangle with a width and a height` | `Rectangle (width, height)` | `Rectangle (w, h)` |

This design ensures patterns remain stable when new fields are added to a variant. If you add `center` to `Circle`, existing patterns matching only `radius` continue to work.

**Partial Matching:**

You may match a subset of fields. Unmatched fields are ignored:

```markdown
If it is a Rectangle (width):
    Return width.
```

**Common Patterns:**

```markdown
A Result is either:
    Success with a value.
    Failure with an error.

An Option is either:
    Some with a value.
    Nothing.
```

### 3.4 Dependent Types

**Pi Types (Π) — Universal Quantification:**

The English "for every X, a Y" becomes Π(x: X). Y:

```markdown
For every Nat n, a sequence of length n.
→ Π(n: Nat). Vec<T, n>
```

**Sigma Types (Σ) — Existential Quantification:**

The English "some X such that P" becomes Σ(x: X). P(x):

```markdown
Some Nat n such that n is prime.
→ Σ(n: Nat). Prime(n)
```

### 3.5 Refinement Types

Refinement types attach predicates to base types:

```markdown
# A Positive Integer

A PositiveInt is an Int where the Int is greater than 0.
```

This creates:
```
PositiveInt = { n: Int | n > 0 }
```

**Implementation Phasing (SMT Integration):**

Full refinement type verification requires an SMT solver (e.g., Z3). This is phased:

| Version | Capability | Compile Time Impact |
|---------|------------|---------------------|
| **v0.5** | Syntactic constants only (`n > 0` where n is literal) | None |
| **v0.5** | Complex refinements become `debug_assert!` | Minimal |
| **v0.6+** | Optional Z3 integration for static verification | Release builds only |

**Rationale:** Sub-second compilation is critical for dev experience. Full SMT verification is deferred to release mode, while dev mode uses runtime assertions.

**With constraints:**
```markdown
# A Valid Email

A ValidEmail is Text where the Text contains "@" and the Text contains ".".
```

#### 3.5.1 Constraint Grammar

Refinement clauses (`where ...`) must use a restricted grammar translatable to SMT solvers:

| Category | Allowed Expressions |
|----------|---------------------|
| Comparisons | `is greater than`, `is less than`, `equals`, `is not`, `is at least`, `is at most` |
| Logic | `and`, `or`, `not` |
| Arithmetic | `plus`, `minus`, `times`, `divided by`, `modulo` |
| Collections | `contains`, `is empty`, `has length` |
| References | Pure functions defined in `## Logic` sections |

**Forbidden:** Side effects, IO operations, imperative statements, mutable references.

**Valid example:**
```markdown
A ValidAge is a Nat where the Nat is at least 0 and the Nat is at most 150.
```

**Invalid example:**
```markdown
A ValidUser is a User where validate(user) returns true.
```
The above is forbidden because `validate` may have side effects.

#### 3.5.2 Runtime Validation (The Inspection Rule)

Refinement types are erased at runtime, but data entering the system from external sources (IO, JSON, databases) is "dirty" — it carries no compile-time proofs.

To upgrade a base type to a refinement type, you must **Inspect** it. The compiler generates the runtime check automatically.

**Definition:**
```markdown
A ValidUser is a User where login_count > 0.
```

**Validation Syntax:**
```markdown
To process (raw_user: User):
    Inspect raw_user as ValidUser:
        If it matches (v):
            Process v.
        Otherwise:
            Return Failure("Validation failed").
```

**Semantics:**

| Step | Action |
|------|--------|
| 1. Parse | Compiler identifies `Inspect X as RefType` pattern |
| 2. Generate | Emits runtime check for refinement constraints |
| 3. Branch | `If it matches` receives the validated type; `Otherwise` handles failure |

**The validated binding `v` has type `ValidUser`** within the success branch. The compiler guarantees the refinement predicate holds.

**Chained Validation:**
```markdown
Inspect raw_data as ValidInput:
    If it matches (input):
        Inspect input.user as ValidUser:
            If it matches (user):
                Return Success(user).
            Otherwise:
                Return Failure("Invalid user").
    Otherwise:
        Return Failure("Invalid input").
```

**Why This Matters:**
This bridges the gap between "dirty" IO data and "clean" internal logic. External data must pass through validation before entering the refined type system.

#### 3.5.3 The Computability Restriction

Refinement types used with `Inspect` (runtime validation) may only use **Computable Predicates** — expressions that can be evaluated in finite time with finite memory.

**Predicate Categories:**

| Category | Examples | Allowed in Inspect? |
|----------|----------|---------------------|
| **Computable** | `x > 0`, `text contains "@"`, `list is empty` | Yes |
| **Logical** | `∀n. P(n)`, `∃x. Q(x)`, quantified statements | No |
| **Opaque** | User-defined predicates with unknown complexity | No (unless marked) |

**The Halting Problem Constraint:**

A refinement type like:
```markdown
A HaltingProgram is a Program where the program halts.
```

Cannot be validated at runtime because the Halting Problem is undecidable.

**Compile-Time vs. Runtime:**

| Construct | Predicate Requirement |
|-----------|----------------------|
| Refinement Type Definition | Any well-formed predicate |
| `Assert that P.` (compile-time proof) | Any predicate (checked by prover) |
| `Inspect X as RefType` (runtime check) | Computable predicates only |
| `Check that P.` (runtime assertion) | Computable predicates only |

**Marking Predicates as Computable:**

User-defined predicates default to "opaque" (non-computable). To enable runtime use:

```markdown
A prime is decidable.
A Nat n is prime where n > 1 and no Int from 2 to n-1 divides n.
```

The `decidable` marker asserts the predicate terminates for all inputs.

**Compiler Enforcement:**

When parsing `Inspect X as RefType`:
1. Extract the refinement predicate from `RefType`
2. Check if all sub-predicates are computable
3. If any sub-predicate is non-computable: Error

**Error Example:**
```
Cannot use 'HaltingProgram' with Inspect — refinement predicate
'program halts' is not computable. Use compile-time proof instead.
```

### 3.6 Generics (The Adjective System)

Instead of angle brackets `<T>`, LOGOS uses natural English adjectives and prepositions:

**Defining a generic type:**
```markdown
# A Stack of [Things]

A Stack is a structure containing:
    a sequence of [Things].
    a count, which is a Nat.

To push (item: [Thing]) onto (stack: Stack of [Things]):
    Append item to the stack's sequence.
    Increase the stack's count by 1.

To pop from (stack: Stack of [Things]):
    If the stack's count equals 0, return nothing.
    Let item be the last element of the stack's sequence.
    Remove the last element from the stack's sequence.
    Decrease the stack's count by 1.
    Return item.
```

**Using a generic type:**
```markdown
Let numbers be a Stack of Integers.
Push 42 onto numbers.
Push 17 onto numbers.
Let top be the result of popping from numbers.
```

The compiler infers `[Thing]` = `Integer` from context.

**Multiple type parameters:**
```markdown
# A Mapping from [Keys] to [Values]

A Mapping contains pairs of [Keys] and [Values].
```

**Bounded generics (constraints):**
```markdown
# A Sortable Collection of [Items]

A SortableCollection requires that [Items] can be compared.
```

**Type Preposition Precedence:**

When nesting generic types, prepositions associate **right-to-left**:

| Expression | Interpretation |
|------------|----------------|
| `List of Sets of Integers` | `List<Set<Integer>>` |
| `Map from Text to List of Users` | `Map<Text, List<User>>` |
| `Stack of Pairs of Keys and Values` | `Stack<Pair<Key, Value>>` |

**Explicit Grouping:**

For complex cases, use parentheses to override default precedence:

```markdown
Map from (List of Keys) to Values.     ← Keys are wrapped in List
(Map from Keys to Values) of Integers. ← Forces different parse
```

**Ambiguity Prevention:**

The parser applies longest-match for type prepositions. If ambiguity remains, the compiler raises an error requesting explicit grouping.

**Type Parsing Grammar (The Disambiguation Rule):**

To resolve the conflict between Generic Types (`Stack of Integers`) and Possessive Nouns (`Owner of House`), the parser uses a specific sub-grammar for Type Definitions that prioritizes Generics.

1. **Registry Lookup:** When parsing a type expression `X of Y`:
   - The compiler checks if `X` is registered as a **Generic Type** in the `TypeRegistry`.
   - If `X` is generic, the expression is parsed as instantiation: `X<Y>`.
   - If `X` is NOT generic, the expression is parsed as possession/field access: `Y.x`.

2. **Conflict Resolution:** The `TypeRegistry` is the final authority. If a user defines `Stack` as a generic type, they cannot use "Stack of Hay" to mean "Hay's Stack" without explicit grouping: `(Stack) of Hay`.

| Expression | TypeRegistry | Interpretation |
|------------|--------------|----------------|
| `Stack of Integers` | `Stack<T>` defined | Generic: `Stack<Integer>` |
| `Owner of House` | `Owner` not generic | Possessive: `owner.house` or relation |
| `List of Items` | `List<T>` defined | Generic: `List<Item>` |

**Scope Exclusivity Rule:**

A noun cannot be both a Generic Type name and a Field Name in the same scope. This constraint prevents parse ambiguity.

| Definition | Status |
|------------|--------|
| `A Stack of [Things]` + field `stack` in another struct | **Allowed** (different scopes) |
| `A Stack of [Things]` + field `Stack` in same module | **Error** |

The compiler raises: *"Name collision: 'Stack' is defined as both a generic type and a field name in scope."*

**Defining Generic Types:**
```markdown
# A Stack of [Things]

A Stack is a structure containing:
    a sequence of [Things].
```

The `[Things]` syntax declares a type parameter. The TypeRegistry records `Stack` as generic.

**Non-Generic Types:**
```markdown
# An Owner

An Owner has:
    a name (Text).
    a house (House).
```

Here `Owner` has no type parameters. `Owner of something` would be parsed as possessive.

#### 3.6.1 Type Expression Grammar (The Disambiguation Rule)

Type expressions and noun phrases share the `X of Y` pattern but have different semantics. The parser uses **context** to determine which grammar to apply.

**Type Contexts (use `parse_type_expression`):**
- After a colon in parameter declarations: `(x: Stack of Integers)`
- After `be a` / `be an` in type annotations: `Let items be a List of Users.`
- In struct field definitions: `a sequence of [Things].`
- In generic bounds: `where [Items] can be compared`

**Value Contexts (use `parse_noun_phrase`):**
- Subject position: `The owner of the house runs.`
- Object position: `Show the color of the sky.`
- After `the` (definite article): `the Stack of papers` (concrete object, not type)

**Grammar Distinction:**

| Grammar | Input | Output |
|---------|-------|--------|
| `parse_type_expression` | `Stack of Integers` | `Type::Generic { base: Stack, param: Int }` |
| `parse_noun_phrase` | `owner of house` | `Expr::FieldAccess { obj: owner, field: house }` |

**Implementation:**

The parser must NOT reuse `parse_noun_phrase` for type parsing. Create a dedicated `parse_type_expression` function that:

1. Checks if the base noun is in `TypeRegistry` as a generic
2. Parses type parameters recursively (for nested generics)
3. Returns `TypeExpr`, not `NounPhrase`

### 3.7 Type Inference

Types are inferred when possible:

```markdown
Let x be 42.                    → x: Int (inferred)
Let name be "Alice".            → name: Text (inferred)
Let users be an empty sequence. → users: Vec<?> (needs context)
```

Explicit annotation when needed:
```markdown
Let users be an empty sequence of Users.
```

### 3.8 Universe Hierarchy

To avoid paradoxes, types form a hierarchy:

| Level | Contains |
|-------|----------|
| `Type₀` | Base types (Bool, Int, Text, ...) |
| `Type₁` | Types of types (the type `Int` has type `Type₀`) |
| `Type₂` | Types of type-types |
| ... | ... |

The hierarchy is implicit in LOGOS; universe polymorphism handles most cases automatically.

---

## 4. English Grammar Specification

### 4.1 Sentence Types

LOGOS distinguishes three illocutionary forces:

| Force | Example | Semantics |
|-------|---------|-----------|
| **Declarative** | "X is Y." | Assertion (proposition) |
| **Imperative** | "Return X." | Command (computation) |
| **Interrogative** | "Is X greater than Y?" | Query (condition) |

#### 4.1.1 The Canonical Phrasebook

LOGOS accepts a strict subset of English. If a sentence does not match a recognized form, the compiler rejects it with "I don't recognize this sentence structure."

**Recognized Sentence Forms:**

| Category | Pattern | Example |
|----------|---------|---------|
| Binding | `Let X be Y.` | `Let count be 0.` |
| Mutation | `Set X to Y.` | `Set count to count plus 1.` |
| Return | `Return X.` | `Return the result.` |
| Conditional | `If P, Q.` / `If P: ... Otherwise: ...` | `If x equals 0, return nothing.` |
| Loop | `Repeat for every X in Y: ...` | `Repeat for every item in list: ...` |
| Ownership | `Give/Show/Let modify X.` | `Give data to processor.` |
| Trust | `Trust that P because Q.` | `Trust that n > 0 because validated.` |
| Assert | `Assert that P.` | `Assert that x is positive.` |
| Check | `Check that P.` | `Check that list is not empty.` |
| Invocation | `[Verb] [arguments].` | `Process the data.` |
| Bare Expression | `x plus 1.` | **Forbidden** |

The EBNF grammar in §14.1 is the authoritative reference. These patterns are not suggestions—they are the only valid imperative constructs.

**Function Invocation Resolution:**

If a sentence starts with a word matching a function name in the `GlobalSymbolTable`, it is parsed as a function invocation. User-defined function names shadow dictionary words during parsing. This resolution occurs in Pass 2 after the `GlobalSymbolTable` is populated during Pass 1 (see §2.3.7).

**Bare Expressions:**

Bare expressions (calculations not assigned, returned, or passed to a function) are forbidden. The compiler raises: *"This calculation is not used. Did you mean 'Set y to x plus 1.'?"*

#### 4.1.2 The Case Convention (Types vs. Predicates)

To disambiguate `is [X]` expressions at the lexical level, LOGOS enforces strict case conventions:

| Category | Case Rule | Example |
|----------|-----------|---------|
| **Types** | PascalCase (Capitalized) | `User`, `ValidEmail`, `Int` |
| **Predicates/Adjectives** | lowercase | `valid`, `empty`, `prime` |

**Disambiguation:**

```markdown
If x is valid.          → Predicate application: valid(x)
If x is User.           → Type check: typeof(x) == User
If x is a valid User.   → Both: valid(x) ∧ typeof(x) == User
```

**Parser Implementation:**

When encountering `is [WORD]`:
1. Check if `WORD` starts with uppercase
2. If uppercase: Check `TypeRegistry` for type match → Type Check
3. If lowercase: Check `Lexicon` for adjective/predicate → Predicate Application

**Benefits:**

- Eliminates expensive runtime type lookups during parsing
- Aligns with natural English conventions (proper nouns capitalized)
- Consistent with Rust's type naming convention

**Internal vs Surface Representation:**

The Case Convention (Types = Capitalized, predicates = lowercase) applies to **source syntax** that users write, not internal representations:

| Layer | Capitalization | Example |
|-------|----------------|---------|
| Source syntax | lowercase predicates | `if x is valid` |
| Internal lexicon | PascalCase lemmas | `"lemma": "Valid"` in lexicon.json |
| FOL output | Capitalized predicates | `Valid(x)` |

The lexicon may store lemmas in PascalCase for symbol uniqueness. The Parser enforces lowercase for surface predicates to distinguish them from Types. The Transpiler capitalizes predicates in FOL output following convention.

**Error on Violation:**

```markdown
A valid is ...          ← ERROR: Type names must be capitalized (use "Valid")
If x is Valid.          ← OK: Type check
```

**The Adjective-Type Collision Rule:**

A lowercase word cannot be both a predicate AND a type name. If needed, use different words:

| Predicate | Type | Allowed |
|-----------|------|---------|
| `valid` | `ValidUser` | Yes |
| `empty` | `Empty` | No — use `EmptyContainer` |

#### 4.1.3 The Equality Rule

The word "is" is **not** used for value equality. Use `equals`.

**The Three Uses of "Is":**

| Syntax | Meaning | Compiles To |
|--------|---------|-------------|
| `x is a User` | Type check | `matches!(x, User { .. })` |
| `x is valid` | Predicate application | `x.is_valid()` |
| `x equals 5` | Value equality | `x == 5` |

**Forbidden:**

| Pattern | Error |
|---------|-------|
| `x is 5` | Use `x equals 5` for value equality |
| `x is [literal]` | Use `x equals [literal]` for value equality |

**Rationale:**

- `is a [Type]` → type check (article + PascalCase = unambiguous)
- `is [adjective]` → predicate (lowercase = unambiguous)
- `equals [value]` → equality (distinct keyword = unambiguous)

**Error Message:**

```
Error at line 12: Use 'equals' for value comparison.

  12 │ If count is 5:
     │          ^^
     │
  = 'is' checks types and predicates, not values.
  = Change to: If count equals 5:
```

### 4.2 Declarative Sentences

Declarative sentences state facts and define predicates:

```markdown
Socrates is a man.                    → Man(socrates)
All men are mortal.                   → ∀x. Man(x) → Mortal(x)
The factorial of 0 is 1.              → factorial(0) = 1
A prime is divisible only by 1 and itself.  → Prime(n) ↔ ...
```

### 4.3 Imperative Sentences

Imperative sentences command computation:

**Let (variable binding):**
```markdown
Let x be 5.
Let result be the factorial of n.
```

**The "be" Disambiguation Rule:**

The word `be` introduces an expression, not a type. To avoid ambiguity between values, types, and constructors:

| Syntax | Meaning | Example |
|--------|---------|---------|
| `Let x be [EXPR].` | Bind x to expression value | `Let x be 5.` |
| `Let x be a new [TYPE].` | Instantiate with default constructor | `Let x be a new User.` |
| `Let x: [TYPE] be [EXPR].` | Type-annotated binding | `Let x: Int be 5.` |
| `Let x be a [TYPE].` | **Ambiguous — Forbidden** | Compile error |

**Rationale:**

`Let x be a User.` is rejected because the parser cannot determine if this is:
- A declaration without initialization (`let x: User;`)
- An implicit default construction (`let x = User::default();`)

Use explicit syntax to disambiguate:
- `Let x be a new User.` → Calls constructor
- `Let x: User be ...` → Type annotation with explicit value

**The Binding-Must-Initialize Rule:**

In imperative code, every `Let` binding MUST include an initializer expression. Type-only declarations are forbidden.

| Pattern | Status |
|---------|--------|
| `Let x be 5.` | Valid |
| `Let x: Int be 5.` | Valid (type-annotated) |
| `Let x be a new User.` | Valid (constructor call) |
| `Let x be a User.` | **Forbidden** (ambiguous) |
| `Let x: User.` | **Forbidden** (uninitialized) |

**Deferred Initialization:**

For conditional initialization, use `Option` with constructor syntax:

```markdown
Let x be an empty Option of Int.
If condition:
    Set x to 5.
```

**Rationale:**

Uninitialized variables violate memory safety. Unlike Rust's uninitialized handling (which uses MaybeUninit), LOGOS requires explicit optionality. The `an empty Option of T` syntax unifies with the collection constructor pattern.

**Set (mutation):**
```markdown
Set x to 10.
Set the first element of list to 0.
```

**Flow-Sensitive Mutability:**

The compiler infers whether a binding requires mutability based on usage within its scope.

| Usage Pattern | Rust Compilation |
|---------------|------------------|
| `Let x be 5.` (never mutated) | `let x = 5;` |
| `Let x be 5.` followed by `Set x to 10.` | `let mut x = 5;` |

This inference is performed at compile time via control flow analysis. The programmer does not need to declare mutability—the compiler observes whether a `Set` command targets the binding anywhere in scope.

**Example:**
```markdown
Let count be 0.
Repeat for every item in list:
    Set count to count plus 1.
Return count.
```

The compiler sees `Set count` and infers `let mut count = 0;` in the generated Rust.

**Module-Boundary Exception:**

Flow-sensitive inference works within a single compilation unit. For module-level or exported variables, separate compilation makes flow analysis undecidable.

| Scope | Mutability Rule |
|-------|-----------------|
| Local variables | Flow-inferred (scan for `Set` usage) |
| Module-level variables | Must be explicitly marked if mutable |
| Exported/public variables | Must be explicitly marked if mutable |

**Syntax for Explicit Mutability:**
```markdown
Let mutable global counter be 0.
```

This explicit marking is only required at module boundaries. Within a function body, flow inference applies.

**Implementation Note:**
The compiler performs a two-pass analysis per function:
1. **Pass 1:** Scan all statements, identify `Set` targets
2. **Pass 2:** Generate code with correct `let` vs `let mut`

**Return:**
```markdown
Return x.
Return nothing.
Return successfully.
```

**Control flow:**
```markdown
If x is greater than y, return x.
Otherwise, return y.

Repeat for every item in the list:
    Show item to the console.

While the queue is not empty:
    Let item be the next element from the queue.
    Process item.
```

#### 4.3.1 Identifier Resolution Order

Unlike Logic Mode, which auto-registers global constants, Imperative Mode requires strict variable resolution to ensure memory safety. When the compiler encounters an identifier, it searches scopes in this specific order:

1. **Local Scope:** Variables bound by `Let` in the current block (checking closest scope first).
2. **Argument Scope:** Parameters defined in the function signature.
3. **Module Scope:** Variables defined at the top level of the current module.
4. **Import Scope:** Public entities from imported modules.

**Error Condition:**
If an identifier is not found in these four scopes, the compiler raises a **"Variable Not Found" error**. It does *not* create a global logical constant.

**Auto-Registration Disabled:**

In Imperative Mode, the parser does NOT auto-register unknown identifiers as global constants. This is a key difference from Logic Mode.

| Mode | Unknown Identifier Behavior |
|------|----------------------------|
| Logic (Declarative) | Auto-register as logical constant in global scope |
| Imperative | Error: "Variable '[name]' not found" |

**Rationale:**

Imperative code manipulates memory. An unresolved identifier likely indicates a typo or missing declaration. Auto-registration would silently create uninitialized variables, violating memory safety.

**Exception:** Proper names beginning with capital letters (e.g., `Alice`, `Database`) may be resolved against imported module aliases before raising an error.

**Function Resolution:**

When parsing an imperative sentence, the parser first checks if the leading verb matches a function name in the `GlobalSymbolTable` (populated during Pass 1). If matched, the sentence is parsed as a function invocation rather than a generic verb phrase.

| Sentence | Resolution | Result |
|----------|------------|--------|
| `Open the door.` (function `open` registered) | GlobalSymbolTable match | Function call |
| `Open the door.` (no function `open`) | Lexicon verb lookup | Standard verb phrase |

This allows user-defined functions to naturally override dictionary words. The precedence order is: GlobalSymbolTable → Lexicon.

**Variable Shadowing Prohibition:**

Variable shadowing is **forbidden** within the same function scope. Defining `Let x...` when `x` is already bound in the current block is a compile error.

**Rationale:**

In English prose, reusing the same name for different things creates ambiguity. Unlike Rust (which allows shadowing), LOGOS prioritizes readability over convenience.

| Pattern | Status |
|---------|--------|
| `Let x be 5. Let x be 10.` | **Error:** "Variable 'x' is already defined in this scope" |
| `Let x be 5. If condition: Let x be 10.` | **Error:** Shadowing in nested scope also forbidden |
| `Let count be 5. Let user count be 10.` | **OK:** Different identifiers |

**Workaround:**

Use `Set` for mutation or choose distinct names:

```markdown
Let x be 5.
Set x to 10.              ← OK: Mutation, not redefinition

Let initial_x be 5.
Let final_x be 10.        ← OK: Different names
```

#### 4.3.2 Logical Assertions in Imperative Blocks

Logical propositions (implications, quantified statements) are forbidden as bare statements in imperative mode. The parser cannot determine if `If x is positive, x squared is positive.` is a control flow branch or a logical truth.

To embed logical assertions in imperative code, use assertion wrappers:

| Pattern | Verification | Generated Code |
|---------|--------------|----------------|
| `Assert that [proposition].` | Compile-time proof obligation | (erased at runtime) |
| `Check that [proposition].` | Runtime validation | `if !proposition { panic!(...) }` |

**Example:**
```markdown
## Main

To process positive numbers (n: Nat):
    Assert that if n is positive, n squared is positive.
    Check that n is greater than 0.
    Return n times n.
```

The `Assert that` wrapper creates a proof obligation verified at compile time. The `Check that` wrapper generates runtime validation code.

**EBNF:**
```ebnf
assert_stmt    = "Assert" "that" proposition "." ;
check_stmt     = "Check" "that" condition "." ;
```

### 4.4 Quantified Expressions

**Universal:**
```markdown
for all x                  → ∀x
for every user             → ∀u: User
for each item in list      → ∀i ∈ list
```

**Existential:**
```markdown
some x                     → ∃x
there exists a user        → ∃u: User
some item in list          → ∃i ∈ list
```

**Generalized:**
```markdown
most users                 → MOST(u: User)
few items                  → FEW(i: Item)
many elements              → MANY(e: Element)
at least 3 users           → |{u: User | P(u)}| ≥ 3
at most 5 items            → |{i: Item | P(i)}| ≤ 5
exactly 2 elements         → |{e: Element | P(e)}| = 2
```

### 4.5 Conditional Expressions

**If-then-otherwise:**
```markdown
If condition, then consequence.
If condition:
    action1.
    action2.
Otherwise:
    alternative.
```

### 4.6 Pattern Matching (The Inspection System)

Pattern matching uses "Inspect" or "Consider":

**Basic pattern matching:**
```markdown
Inspect the value:
    If it is nothing, return 0.
    If it is some x, return x.
```

**Exhaustive matching with variants:**
```markdown
Inspect the server response:
    If it is Success(data):
        Show data to the console.
        Return data.
    If it is Error(code, message):
        Log "Error [code]: [message]".
        Return the error.
    Otherwise:
        Panic with "Unknown response type".
```

**Nested pattern matching:**
```markdown
Inspect the result:
    If it is Success(User(name, age)):
        Show "Found user [name], age [age]".
    If it is Success(Guest):
        Show "Anonymous guest".
    If it is Failure(reason):
        Show "Failed: [reason]".
```

**Guard clauses:**
```markdown
Inspect the number:
    If it is n where n is greater than 100:
        Return "large".
    If it is n where n is greater than 10:
        Return "medium".
    Otherwise:
        Return "small".
```

### 4.7 Ownership Verbs

**Give (move ownership):**
```markdown
Give the data to the processor.
```
Compiles to: `processor.process(data)` where `data` is moved.

**Show (immutable borrow):**
```markdown
Show the data to the validator.
```
Compiles to: `validator.validate(&data)`

**Let modify (mutable borrow):**
```markdown
Let the sorter modify the list.
```
Compiles to: `sorter.sort(&mut list)`

**Copy:**
```markdown
Give a copy of the data to the processor.
```
Compiles to: `processor.process(data.clone())`

### 4.8 Concurrency Phrases

**Structured concurrency:**
```markdown
Attempt all of the following:
    Fetch user data.
    Log the request.
Then continue with the response.

Await the first success of:
    Query primary database.
    Query backup database.

Simultaneously:
    Process batch A.
    Process batch B.
```

**Agent communication:**
```markdown
Give the record to the DatabaseAgent.
Ask the CacheAgent for the value of key.
Await the response or timeout after 5 seconds.
```

---

## 5. Totality & Termination

### 5.1 The Halting Principle

All functions in LOGOS must be **total** (guaranteed to halt). Infinite loops are only allowed in specific `System` contexts (e.g., server main loops marked with `forever`).

### 5.2 Smart Inference

For most loops, the compiler infers the termination condition automatically:

| Pattern | Inference |
|---------|-----------|
| `Repeat for i from 1 to 10` | i increases to bound |
| `Repeat for every item in list` | list length is finite |
| `Recursion on tail of list` | list size decreases |

### 5.3 Explicit Decreasing Variants

For complex loops where inference fails, provide a `decreasing` annotation:

```markdown
While x is greater than 0 (decreasing x):
    Set x to x minus 1.
    If x is even:
        Set x to x divided by 2.
```

If the compiler cannot prove the variant decreases in a well-founded ordering, it raises a **Totality Error** with a Socratic prompt asking for the annotation.

---

## 6. The Socratic Error System

### 6.1 Philosophy

Errors are not crashes; they are deviations from the plan. When a program fails, LOGOS provides a **post-mortem investigation** containing three components.

### 6.2 The `Failure` Type

The standard `Result` is exposed as `Success(value)` or `Failure(error)`.
A `Failure` object automatically captures:

| Component | Description |
|-----------|-------------|
| **Message** | Human-readable description of what went wrong |
| **Story** | Reverse-narrative trace mapping call stack to English sentences |
| **State** | (Dev mode) Snapshot of relevant variable values at failure |

### 6.3 Example Error Output

```
**Computation Failed**

**The Message:**
I could not find the file at "/tmp/data.txt".

**The Story:**
1. You asked to "Load the user configuration" (Config.md, line 45).
2. Which tried to "Read the contents of file at path" (IO.md, line 12).
3. The system reported that the path does not exist.

**The State:**
- path: "/tmp/data.txt"
- user_id: 42

**Suggestion:**
- Does the file exist at that path?
- Did you mean to use "Create the file if missing"?
```

### 6.4 Propagation

Errors propagate automatically through function calls. The `Story` accumulates context at each level. Use `try` to explicitly handle:

```markdown
Let result be try reading the file at path.
If result is Failure:
    Log "File missing, using defaults.".
    Return the default config.
```

---

## 7. Proof System

### 7.1 Curry-Howard Correspondence

In LOGOS, the correspondence is explicit:

| Logic | Programming |
|-------|-------------|
| Proposition | Type |
| Proof | Term (program) |
| Implication (A → B) | Function type (A → B) |
| Conjunction (A ∧ B) | Product type (A, B) |
| Disjunction (A ∨ B) | Sum type (Either A B) |
| Universal (∀x. P(x)) | Dependent function (Π(x: T). P(x)) |
| Existential (∃x. P(x)) | Dependent pair (Σ(x: T). P(x)) |
| True | Unit type |
| False | Empty type (Void) |

### 7.2 Theorem Syntax

```markdown
> **Theorem [name]:** [proposition in English]
>
> *Proof:* [proof in English or tactic]
```

Example:
```markdown
> **Theorem addition_identity:** For all natural numbers n, n plus 0 equals n.
>
> *Proof:* By induction on n.
> - **Base case:** When n is 0, 0 plus 0 equals 0 by the definition of addition.
> - **Inductive step:** Assume the property holds for k. We must show
>   (k + 1) plus 0 equals k + 1. By definition, (k + 1) plus 0 equals
>   (k plus 0) plus 1. By the inductive hypothesis, k plus 0 equals k.
>   Therefore, (k + 1) plus 0 equals k + 1.
```

### 7.3 Proof Tactics

When full English proofs are verbose, tactics provide automation:

| Tactic | Meaning |
|--------|---------|
| `Auto` | Attempt automatic proof search |
| `By definition` | Unfold definitions |
| `By hypothesis` | Apply an assumption |
| `By [theorem name]` | Apply a previously proven theorem |
| `By induction on x` | Structural induction |
| `By contradiction` | Assume negation, derive False |
| `By cases on x` | Case analysis |

Example with tactics:
```markdown
> **Lemma zero_plus:** 0 plus n equals n for all n.
> *Proof:* By definition of addition. Auto.
```

### 7.4 Proof Obligations

Certain operations generate automatic proof obligations:

**Division:**
```markdown
Let result be x divided by y.
```
Generates: Obligation to prove `y ≠ 0`.

**Array access:**
```markdown
Let element be the item at index i in list.
```
Generates: Obligation to prove `0 ≤ i < length(list)`.

**Refinement construction:**
```markdown
Let p be a PositiveInt from x.
```
Generates: Obligation to prove `x > 0`.

### 7.5 Proof Irrelevance

**Principle:** Proofs are erased at runtime, like Rust erases lifetimes.

This means:
1. **Compile-time verification:** All proofs are checked during type checking
2. **Zero runtime overhead:** The compiled binary contains no proof data
3. **Proof erasure:** Different proofs of the same proposition compile to identical code

**Example:**
```markdown
> **Theorem:** n plus 0 equals n.
> *Proof:* By induction on n. [100 lines of proof]

> **Theorem:** n plus 0 equals n.
> *Proof:* Auto.
```

Both compile to the same binary. The proof is never executed.

**Debug mode exception:**
In development builds, proof obligations become `debug_assert!` statements for runtime checking. In release builds, they are erased entirely.

### 7.6 Totality Checking

All functions must be total (terminate on all inputs). The compiler verifies:

1. **Structural recursion** — Recursive calls on structurally smaller arguments
2. **Well-founded recursion** — Calls decrease according to a well-founded measure
3. **Explicit termination proofs** — User-provided termination arguments

```markdown
> **Theorem termination:** The factorial function terminates for all natural numbers.
> *Proof:* The recursive call uses n - 1, which is structurally smaller than n
> for all n > 0. The base case handles n = 0. Auto.
```

### 7.7 Manual Override (The Trust Verb)

When the SMT solver cannot prove a proposition that the programmer knows is true, use the **Trust** verb with mandatory justification:

```markdown
Trust that [proposition] because [reason].
```

**Example:**
```markdown
## To Calculate Inverse Square

To calculate the intensity for (distance: Real):
    Trust that distance is non-zero because physical objects occupy space.

    Let result be 1.0 divided by (distance squared).
    Return result.
```

**Semantics:**

| Step | Action |
|------|--------|
| **Assumption** | Adds the proposition to the local proof context (Γ = Γ ∪ {P}) |
| **Audit** | Emits a compiler warning containing the justification |
| **Erasure** | Generates zero runtime code |

**The `because` clause is mandatory.** The compiler rejects bare `Trust that P.` statements without justification.

**Auditing Trust statements:**
```bash
logos audit program.md    # List all Trust statements with justifications
```

**Lifting Variables (Snapshotting):**

When an imperative variable is referenced inside a `Trust` block, Refinement Type, or Proof obligation, it is **lifted** into the logical context as a **snapshot** of its value at that specific line of code.

| Concept | Semantics |
|---------|-----------|
| **Snapshot** | The variable's value at the Trust statement is captured |
| **Immutability** | The lifted value is treated as a constant for verification |
| **Scope** | The snapshot is valid only within the Trust's logical scope |

**Example:**
```markdown
Let x be 5.
Trust that x equals 5 because it was just assigned.
Set x to 10.
Trust that x equals 10 because it was reassigned.
```

Both Trust statements are valid. Each captures `x` at its current value.

**Caveat:** Referencing a variable that may be uninitialized at the Trust point is a compile error.

**Implementation:**

The compiler generates SSA (Static Single Assignment) form internally. Each `Trust` references a specific SSA version of the variable.

---

## 8. Memory Model

### 8.1 Ownership Semantics

LOGOS adopts Rust's ownership model, expressed through English verbs:

| English | Rust | Semantics |
|---------|------|-----------|
| "Give X to F" | `f(x)` | Move: F takes ownership of X |
| "Show X to F" | `f(&x)` | Borrow: F reads X, caller keeps ownership |
| "Let F modify X" | `f(&mut x)` | Mutable borrow: F can change X |
| "Copy X" | `x.clone()` | Clone: Create independent copy |

#### 8.1.1 Minimal Ownership Tracking (v0.5 Stub)

Before full borrow checking, the compiler implements a simple state machine for immediate error feedback:

```rust
enum OwnershipState {
    Owned,    // Variable holds exclusive ownership
    Moved,    // Ownership transferred via "Give"
    Borrowed, // Temporarily shared via "Show"
}
```

**Compiler Behavior:**

| Event | Action | Symbol Table Update |
|-------|--------|---------------------|
| `Let x be ...` | Create binding | `x → Owned` |
| `Give x to f.` | Transfer ownership | `x → Moved` |
| `Show x to f.` | Create reference | `x → Borrowed` (temporary) |
| Use of `x` after `Give` | **Error** | "You already gave 'x' away on line N" |

**Example Error:**
```
Error at line 15: You already gave 'data' away.

  13 │ Let data be the user's profile.
  14 │ Give data to the database.
  15 │ Show data to the console.
     │      ^^^^
     │
  = You gave 'data' to 'database' on line 14.
  = Once you give something away, you can no longer use it.
  = Did you mean to show it first, then give it?
```

**Limitation:** This stub catches use-after-move but not complex borrow patterns (nested lifetimes, mutable aliasing). Full lifecycle analysis deferred to v0.6+.

### 8.2 Ownership Rules

1. **Single Owner:** Every value has exactly one owner
2. **Move Semantics:** Giving a value transfers ownership
3. **Borrow Checking:** References must not outlive their referent
4. **Exclusive Mutation:** At most one mutable reference at a time

### 8.3 Examples

**Move:**
```markdown
Let data be the user's profile.
Give data to the database.
Show data to the console.  ← COMPILE ERROR: data was moved
```

**Borrow:**
```markdown
Let data be the user's profile.
Show data to the validator.
Show data to the console.  ← OK: data was only borrowed
```

**Mutable borrow:**
```markdown
Let data be a sequence of Integers.
Let the sorter modify data.
Show data to the console.  ← OK: mutable borrow ended
```

**Note on Mutability:**
The compiler infers mutability via flow analysis (see §4.3). A binding becomes mutable if it is the target of a `Set` command or passed to a function via `Let modify`.

**Field Access with Mutable References:**

If you have a mutable reference to a record (via `Let modify`), you may `Set` any of its fields:

```markdown
Let the processor modify the user.
Set the user's login_count to 0.
```

The mutation is scoped to the lifetime of the mutable borrow.

### 8.4 Lifetime Inference

Lifetimes are inferred from English scope:

```markdown
To process with a reference to data:
    Show data to the analyzer.
    Return the analysis result.
```

The reference `data` is valid for the duration of `process`. The compiler infers appropriate lifetime bounds.

### 8.5 The Zone System (Manual Memory)

For performance-critical code, LOGOS provides **Zones** — arena-based allocators with automatic cleanup.

**Philosophy:** "What happens in the Zone, stays in the Zone."

**Creating and using a Zone:**
```markdown
## To Render Frame

To render the scene:
    Let canvas be the main display.
    Inside a new zone called "ScratchSpace":
        Let particles be 5000 Particle objects.
        Repeat for every p in particles:
            Update p's position.
            Draw p onto canvas.
    Log "Frame render complete".
```

The zone is automatically freed when exiting the indented block.

**Zone properties:**
1. **Bump allocation:** Objects allocate sequentially (O(1))
2. **No individual deallocation:** Objects cannot be freed one-by-one
3. **Automatic bulk free:** All objects freed when block exits
4. **Escape prevention:** References cannot escape the zone's scope

**Performance characteristics:**
| Operation | Standard Heap | Zone |
|-----------|---------------|------|
| Allocate | O(log n) | O(1) |
| Deallocate | O(log n) | N/A |
| Bulk free | O(n) | O(1) |

**Compiles to (Rust):**
```rust
let canvas = get_main_display();
let arena = bumpalo::Bump::new();
{
    let particles: &mut [Particle] = arena.alloc_slice_fill_default(5000);
    for p in particles.iter_mut() {
        update_position(p);
        draw(p, &canvas);
    }
} // arena dropped here, all memory freed instantly
println!("Frame render complete");
```

**The Containment Rule (Hotel California):**

**References** to zone-allocated data cannot escape the zone. **Values** (Copy types) can escape freely. This rule is enforced at compile time.

| Data Type | Can Escape? | Mechanism |
|-----------|-------------|-----------|
| Primitives (Int, Bool, Char) | Yes | Implicit copy (Copy trait) |
| References to zone data | No | Compile error |
| Non-Copy structs | Yes, via explicit clone | `a copy of X` |
| Zone-allocated pointers | No | Compile error |

**Values Escape Freely:**
```markdown
Let total be 0.
Inside a new zone:
    Repeat for every item in huge_list:
        Set total to total plus item.
Return total.  ← OK: total is an Int (Copy type)
```

**References Cannot Escape:**
```markdown
Let result be nothing.
Inside a new zone:
    Let temp be a huge calculation.
    Set result to temp.  ← ERROR: Reference cannot escape
```

This produces a compile error: "Reference 'temp' cannot escape zone. The object will be deallocated when the zone exits."

**Explicit Copy for Non-Copy Types:**
```markdown
Let result be nothing.
Inside a new zone:
    Let temp be a huge calculation.
    Set result to a copy of temp.  ← OK: Deep clone to heap
```

The explicit `a copy of` creates a heap-allocated clone that survives the zone.

**Lifetime Semantics:**

The compiler tags all zone-allocated references with a lifetime `'zone`. Any attempt to store a `'zone` reference in a location with a longer lifetime triggers the Containment Rule error. Copy types bypass this check because they are duplicated, not referenced.

**Zone Return Prohibition:**

Zone-allocated data cannot be **returned** from the function that created the zone. The zone is destroyed when its block exits, so any reference would be dangling.

| Operation | Allowed? | Reason |
|-----------|----------|--------|
| Return Copy type from zone | Yes | Value is duplicated |
| Return reference to zone data | No | Reference would dangle |
| Assign zone data to outer variable | No | Outlives zone scope |
| Return explicit clone | Yes | Heap-allocated copy |

**Example:**

```markdown
To process items:
    Inside a new zone:
        Let results be compute heavy data.
        Return results.             ← ERROR: Cannot return zone reference

To process items:
    Inside a new zone:
        Let results be compute heavy data.
        Let output be a copy of results.
    Return output.                  ← OK: Clone escapes before zone ends
```

**Copy Semantics and Auto-Derivation:**

The `TypeRegistry` must track whether each type is `Copy` (value semantics) or `Move` (ownership semantics). This is essential for Zone escape analysis.

**Auto-Derivation Rules:**

| Type Definition | Copy Status | Reason |
|-----------------|-------------|--------|
| Primitives (`Int`, `Bool`, `Char`, `Real`) | Copy | Intrinsic |
| Struct with all Copy fields | Copy | Auto-derived |
| Struct with any non-Copy field | Move | Cannot copy |
| Enum with all Copy variants | Copy | Auto-derived |
| References (`borrowed X`) | Copy | Pointer copy |
| Mutable references (`mutable X`) | Move | Exclusive access |
| Zone-allocated data | Move | Lifetime-bound |

**Explicit Opt-Out:**

A struct can be forced to Move semantics even if all fields are Copy:

```markdown
# A Handle

A Handle is NOT copyable.
A Handle has:
    an id (Int).
```

This is useful for types that represent unique resources (file handles, connection IDs).

**Implementation:**

When the compiler encounters a struct definition, it:
1. Checks if all fields are Copy
2. If yes, marks struct as Copy in TypeRegistry
3. If no, marks struct as Move
4. Explicit `NOT copyable` annotation overrides auto-derivation

Zone escape analysis queries this flag to determine if a value can be returned from a zone block.

**TypeRegistry Requirements:**

The TypeRegistry must track both `Copy` (for Zone escape) and `Portable` (for Agent Send):

| Field | Purpose | Used By |
|-------|---------|---------|
| `copy: bool` | Zone escape analysis | Zone containment checker |
| `portable: bool` | Cross-agent serialization | Agent communication verifier |

A complete TypeDef includes: name, fields, generic parameters, copy status, and portable status.

#### 8.5.1 Nested Zone Containment (Strict Stack Discipline)

The Containment Rule extends to **nested zones**. Data allocated in an inner zone cannot be assigned to any variable declared in an outer zone.

**The Nesting Violation:**

```markdown
Inside a new zone "Outer":
    Let x be a Thing.
    Inside a new zone "Inner":
        Let y be a Thing.
        Set x to y.         ← ERROR: Assigning Inner reference to Outer variable
```

**Compiler Error:**
```
Zone safety violation: 'y' has lifetime 'Inner' which is shorter than 'x' (lifetime 'Outer').
Assigning shorter-lived data to longer-lived variable causes use-after-free.
```

**Lifetime Hierarchy:**

Each zone introduces a new lifetime level. Inner zones have strictly shorter lifetimes:

```
Function Scope ('fn)
└── Zone "Outer" ('outer)
    └── Zone "Inner" ('inner)
```

**The Strict Stack Discipline Rule:**

An assignment `Set [TARGET] to [VALUE]` is valid only if:
```
lifetime(VALUE) ≥ lifetime(TARGET)
```

| Assignment | Target Lifetime | Value Lifetime | Valid? |
|------------|-----------------|----------------|--------|
| `Set x to y` | 'outer | 'inner | No |
| `Set y to x` | 'inner | 'outer | Yes |
| `Set local to zone_val` | 'fn | 'zone | No |
| `Set zone_val to local` | 'zone | 'fn | Yes |

**Returning from Nested Zones:**

```markdown
Inside a new zone "Outer":
    Let result be nothing.
    Inside a new zone "Inner":
        Let computed be expensive calculation.
        Set result to a copy of computed.   ← OK: Copy to heap
    Return result.                          ← ERROR: result is in "Outer"
```

The correct pattern:
```markdown
Let result be nothing.
Inside a new zone "Outer":
    Inside a new zone "Inner":
        Let computed be expensive calculation.
        Set result to a copy of computed.   ← OK: result outlives both zones
Return result.                              ← OK
```

### 8.6 Ownership and Concurrency

Ownership enables safe concurrency:

```markdown
Attempt all of the following:
    Show data to analyzer A.  ← Immutable borrow
    Show data to analyzer B.  ← Immutable borrow (OK: shared)
```

```markdown
Attempt all of the following:
    Let modifier A modify data.
    Let modifier B modify data.  ← COMPILE ERROR: concurrent mutable borrows
```

---

## 9. Concurrency Model

### 9.1 Three-Layer Architecture

| Layer | Model | Scope | Verification |
|-------|-------|-------|--------------|
| **Core** | Structured Concurrency | Local/single-machine | Full proof capability |
| **Channels** | CSP Pipelines | Local data flow | Type-checked |
| **Agents** | Actor Model | Distributed/network | Contract-based |

### 9.2 Structured Concurrency (Core)

Structured concurrency ensures tasks form a tree: parents wait for children.

**Fork-Join (Attempt all):**
```markdown
Attempt all of the following:
    Fetch user profile.
    Fetch user preferences.
    Fetch user history.
Then merge the results.
```

Semantics: All three tasks run concurrently. Execution continues only when all complete.

**Race (Await first):**
```markdown
Await the first success of:
    Query primary server.
    Query backup server.
Use whichever responds first.
```

Semantics: Both tasks start. First success wins; other is cancelled.

**Timeout:**
```markdown
Await the result of the query or timeout after 30 seconds.
```

**Cancellation:**
If a parent scope exits (returns, throws), all child tasks are automatically cancelled.

### 9.3 Proof Obligations for Concurrency

Structured concurrency enables verification:

1. **No data races:** Ownership rules prevent concurrent mutable access
2. **No deadlocks:** Tree structure prevents circular waits
3. **Guaranteed cleanup:** Parent completion implies child completion

### 9.4 Channels and Pipelines (CSP)

For data flow patterns, LOGOS provides streams and channels:

**Creating a stream:**
```markdown
Create a stream called "LogStream".
```

**Producing to a stream:**
```markdown
Pour "User connected" into "LogStream".
Pour the error message into "ErrorStream".
```

**Consuming from a stream:**
```markdown
Spawn a background task consuming "LogStream":
    Repeat for every message in "LogStream":
        Write message to the log file.
```

**Pipelines:**
```markdown
Create a pipeline from "RawData" to "ProcessedData" to "Output".
Let numbers flow through the pipeline, transformed by:
    First, parse each item as an integer.
    Then, filter items greater than 0.
    Finally, multiply each by 2.
```

**Compiles to (Rust):**
```rust
let (tx, rx) = tokio::sync::mpsc::channel(100);
tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
        write_to_log(&msg).await;
    }
});
tx.send("User connected").await?;
```

### 9.5 Agent Model (Distributed)

For distributed systems, LOGOS provides Agents—isolated entities that communicate via message passing. Agents can be local (same process) or remote (network).

**Defining an Agent:**
```markdown
# The Cache Agent

The CacheAgent maintains a mapping from keys to values.

The CacheAgent accepts:
    GetRequests, which have a key.
    SetRequests, which have a key and a value.

To handle a GetRequest:
    Look up the key in the mapping.
    Reply with the value or nothing.

To handle a SetRequest:
    Store the value at the key.
    Reply with confirmation.
```

**Agent Instantiation and Lifecycle:**

Agents are not singletons. You can create multiple independent instances of the same agent type.

**Creating an Instance (Handle):**
```markdown
Let my_cache be a new CacheAgent.
Let user_cache be a new CacheAgent.
```

Each `new` expression creates an independent agent instance with its own state. The variable holds a **handle** to that instance.

**Starting an Agent:**
```markdown
Spawn my_cache.
```

`Spawn` schedules the agent to begin processing messages. Before spawning, you may configure the instance:

```markdown
Let my_cache be a new CacheAgent.
Set my_cache's capacity to 1000.
Spawn my_cache.
```

**Communicating with a Specific Instance:**
```markdown
Give request to my_cache.
Ask user_cache for the value at "session:123".
```

The handle identifies which instance receives the message.

**Singleton Fallback:**

If you use `Spawn the [AgentType]` without assignment, a global singleton is created:

```markdown
Spawn the CacheAgent.
Give request to the CacheAgent.
```

This is convenient for system-wide services but limits you to one instance per type.

| Syntax | Semantics |
|--------|-----------|
| `Let x be a new Agent.` | Create instance, assign handle |
| `Spawn x.` | Start instance |
| `Give msg to x.` | Send to specific instance |
| `Spawn the Agent.` | Create global singleton |
| `Give msg to the Agent.` | Send to singleton |

**The Portable Constraint:**

Data sent to Agents must be `Portable`. This is enforced at compile time.

| Portable | Not Portable |
|----------|--------------|
| Primitives (Int, Text, Bool) | File handles |
| Records of Portable types | Mutex/Lock references |
| Enums of Portable types | Zone-allocated references |
| Sequences/Maps of Portable types | Raw pointers |

Attempting to send a non-Portable type produces a compile error: "Type 'FileHandle' is not Portable and cannot be sent to an Agent."

**Portable Trait Definition:**

`Portable` is a **marker trait** (similar to Rust's `Send` or `Sync`). The compiler derives it automatically based on structural rules:

| Type | Portable? | Reason |
|------|-----------|--------|
| Primitives (`Int`, `Bool`, `Text`, `Char`) | Yes | Intrinsically copyable |
| Records (all fields Portable) | Yes | Derived from fields |
| Records (any field non-Portable) | No | Transitive failure |
| Enums (all variants Portable) | Yes | Derived from variants |
| Zone-allocated references | No | Cannot escape zone lifetime |
| File handles, Mutexes, raw pointers | No | OS resources are not serializable |
| Generic `T` | Depends | Portable iff T is Portable |

**Explicit Opt-Out:**

To mark a type as non-Portable even if structurally eligible:

```markdown
# A Session Token

A SessionToken is NOT portable.
A SessionToken has:
    a value (Text).
```

**Verification Phase:**

The compiler checks Portability during the **Type Checking** phase, before code generation. A violation produces: "Type '[T]' is not Portable and cannot be sent to Agent '[A]'."

### 9.6 Communication Verbs: Give vs Send

LOGOS distinguishes between local ownership transfer and remote message passing.

**Give (Local Move):**

Used for local agents in the same memory space.

```markdown
Give the record to the LocalWorker.
```

| Property | Value |
|----------|-------|
| Semantics | Ownership transfer (pointer move) |
| Performance | Instant (no serialization) |
| Fallibility | Infallible |
| Portability | Not required (can pass non-Portable types) |

**Send (Remote Copy):**

Used for remote agents or when keeping the original data.

```markdown
Send the message to the RemoteDatabase.
```

| Property | Value |
|----------|-------|
| Semantics | Serialization + network transmission |
| Performance | Slow (network round-trip) |
| Fallibility | Returns Result/Future |
| Portability | Required (must be Portable) |

**Verb Enforcement:**

Using `Give` with a remote Agent is a compile error:

```markdown
Give the record to the RemoteAgent.
```

Error: "Cannot 'Give' to remote Agent 'RemoteAgent'. Use 'Send' for network communication."

**Handling Send Failures:**

Because `Send` involves the network, it returns a Result:

```markdown
Attempt to send the query to RemoteDB.
If delivery fails:
    Log "Network error: [error]".
    Retry after 1 second.
Otherwise:
    Process the response.
```

**Summary:**

| Verb | Scope | Ownership | Fallible | Portable Required |
|------|-------|-----------|----------|-------------------|
| Give | Local | Transfer | No | No |
| Send | Remote | Copy | Yes | Yes |

**Boundary Validation:**

When an Agent receives a message containing Refinement Types, the runtime automatically executes the verification logic. If the logical constraints (e.g., `where n > 0`) are not satisfied, the message is rejected as a `Failure` before the handler code runs.

This ensures that refinement guarantees hold across serialization boundaries, even though proofs are erased during normal compilation.

### 9.7 Agent Contracts

Agents can specify contracts (pre/post conditions):

```markdown
# The Validator Agent

The ValidatorAgent validates user records.

> **Contract:** Given a UserRecord, the ValidatorAgent returns either
> a ValidatedUser or a sequence of ValidationErrors.
>
> **Precondition:** The UserRecord has non-empty name and email fields.
> **Postcondition:** If successful, the ValidatedUser satisfies all business rules.
```

---

## 10. Standard Library

### 10.1 Core Types

| English Name | Type | Rust |
|--------------|------|------|
| a sequence of X | `Seq<X>` | `Vec<X>` |
| a mapping from K to V | `Map<K, V>` | `HashMap<K, V>` |
| an optional X | `Option<X>` | `Option<X>` |
| either X or an error | `Result<X>` | `Result<X, Error>` |
| a set of X | `Set<X>` | `HashSet<X>` |

### 10.2 Numeric Tower

```
Nat ⊂ Int ⊂ Rat ⊂ Real ⊂ Complex
```

| Type | Description | Rust |
|------|-------------|------|
| `Nat` | Natural numbers (0, 1, 2, ...) | `u64` / `BigUint` |
| `Int` | Integers (..., -1, 0, 1, ...) | `i64` / `BigInt` |
| `Rat` | Rationals (p/q) | Custom |
| `Real` | Real numbers | `f64` |
| `Complex` | Complex numbers | Custom |

**Integration with Mixed Math Syntax:**

The Numeric Tower types (`Nat`, `Int`, `Rat`, `Real`, `Complex`) support both prose and symbolic arithmetic. For complex expressions with mixed operators, use **backtick syntax** (Section 2.10) to invoke standard mathematical precedence (PEMDAS).

| Expression Type | Syntax | Precedence |
|-----------------|--------|------------|
| Prose (monotonic) | `a plus b plus c` | Left-to-right, same class only |
| Symbolic (PEMDAS) | `` `a + b * c` `` | Standard: × before + |
| Mixed | `a plus` `` `b * c` `` | Backticks group subexpression |

**Example:**

```markdown
Let kinetic_energy be `0.5 * m * v²`.
Let total be sum plus `delta * factor`.
Let count be a plus b plus c.
```

The Monotonic Rule (Section 2.11) ensures that prose arithmetic remains unambiguous. Use backticks as the "math mode" escape when standard precedence is required.

### 10.3 Sequence Operations

```markdown
Let length be the length of list.
Let first be the head of list.
Let rest be the tail of list.
Let combined be list1 appended with list2.
Let slice be items 2 through 5 of list.
Let mapped be each item in list transformed by f.
Let filtered be items in list where condition holds.
Let total be list reduced by addition starting from 0.
```

### 10.4 Mapping Operations

```markdown
Let value be the value at key in map.
Set the value at key in map to newValue.
Remove the entry for key from map.
Let keys be all keys in map.
Let values be all values in map.
```

### 10.5 IO Operations

```markdown
Show message to the console.
Read a line from the console.
Read the contents of file at path.
Write contents to file at path.
```

### 10.6 FFI Conventions

**Rust interop:**
````markdown
```rust
fn external_function(x: i64) -> i64 {
    x * 2
}
```

To double x:
    Call the Rust function external_function with x.
````

**C interop:**
````markdown
```c
extern int printf(const char* format, ...);
```

To print formatted text:
    Call the C function printf with the format string.
````

**Macro Restriction:**

FFI can only call Rust **functions**, not **macros**. Rust macros use syntax incompatible with LOGOS parsing.

| Callable | Example | Status |
|----------|---------|--------|
| Rust functions | `fn compute(x: i64) -> i64` | Allowed |
| Rust macros | `println!("Hello")` | Not allowed |
| C functions | `extern int printf(...)` | Allowed |
| C macros | `#define MAX(a,b) ...` | Not allowed |

**Workaround:**
To use functionality provided by macros, wrap them in functions:

```rust
fn log_message(msg: &str) {
    println!("{}", msg);
}
```

Then call the wrapper function from LOGOS:
```markdown
Call the Rust function log_message with the message.
```

### 10.6.1 Type Marshaling

When crossing the FFI boundary, types marshal as follows:

| LOGOS Type | Rust Type | C Type | Notes |
|------------|-----------|--------|-------|
| `Nat` | `u64` | `uint64_t` | Unsigned 64-bit integer |
| `Int` | `i64` | `int64_t` | Signed 64-bit integer |
| `Text` | `String` | `char*` | UTF-8; C requires null terminator |
| `Bool` | `bool` | `_Bool` | Native boolean |
| `Char` | `char` | `uint32_t` | Unicode code point |
| `Seq of [T]` | `Vec<T>` | `T*` + `size_t` | Pointer + length for C |
| `Option of [T]` | `Option<T>` | Tagged union | Nullable pointer optimization applies |
| `Map from [K] to [V]` | `HashMap<K,V>` | N/A | No direct C equivalent |
| `[T] where ...` | `T` | `T` | Refinements erased at runtime |

**Refinement Erasure:** Refinement types compile to their base type. The constraint is verified at compile-time but generates no runtime code.

**Generic Type Restriction:**

FFI calls are restricted to **concrete types** only. Generic types cannot be safely marshaled because the compiler cannot guarantee memory layout of type parameters at the FFI boundary.

| Pattern | Status |
|---------|--------|
| `Call rust_fn with (x: Int).` | Allowed |
| `Call rust_fn with (list: Seq of Int).` | Allowed (concrete) |
| `Call rust_fn with (list: Seq of [T]).` | **Forbidden** (generic) |

**Workaround:**

For generic FFI needs, create monomorphized wrappers in `logos_core`:

```rust
fn process_int_list(list: Vec<i64>) -> Vec<i64> { ... }
fn process_text_list(list: Vec<String>) -> Vec<String> { ... }
```

Then call the appropriate concrete wrapper from LOGOS.

**C String Interoperability:**

LOGOS `Text` uses Rust's length-prefixed `String`. C functions expect null-terminated `char*`. The compiler injects conversion code at C FFI boundaries:

```rust
let c_string = CString::new(text).unwrap();
unsafe { c_func(c_string.as_ptr()) };
// c_string is automatically dropped here
```

**Performance Consideration:** Each C FFI call involving strings incurs a heap allocation for the null-terminated copy. For performance-critical code calling C functions in tight loops, consider:
1. Caching the converted `CString` outside the loop
2. Using a Zone allocator for temporary C strings
3. Passing pre-converted strings from Rust FFI instead

#### 10.6.2 String Marshaling Safety

LOGOS `Text` values are UTF-8 strings with immutability guarantees. C functions that modify string buffers require explicit `Buffer` types.

**The Read-Only Default Rule:**

When passing `Text` to C:
1. The string is passed as `const char*` (read-only pointer)
2. The C function signature must declare the parameter as `const`
3. Attempting to write through this pointer is undefined behavior

**FFI String Types:**

| LOGOS Type | C Equivalent | Mutability | Use Case |
|------------|--------------|------------|----------|
| `Text` | `const char*` | Read-only | Passing strings to C |
| `Buffer` | `char*` + length | Mutable | C functions that write |
| `CString` | `char*` (null-term) | Owned | Returning strings from C |

**Mutable String Buffers:**

When C needs to write to a string:

```markdown
To read file contents:
    Let buffer be a new Buffer of 4096 bytes.
    Call the C function read_file with file_path and a mutable buffer.
    Return the text from buffer.
```

**Buffer Semantics:**

A `Buffer` is:
1. A pre-allocated byte array with known capacity
2. Passed to C as `char*` (mutable pointer) + length
3. Must be explicitly converted to `Text` after C returns

**Compiler Enforcement:**

If a C function signature declares a non-const `char*` parameter, the LOGOS caller must provide a `Buffer`, not a `Text`:

```c
// C header
void modify_string(char* buffer, size_t len);  // Mutable
const char* get_string();                       // Read-only return
```

```markdown
// LOGOS
Let buf be a new Buffer of 256 bytes.
Call modify_string with a mutable buf.          ← OK

Call modify_string with "hello".                ← ERROR: Text is immutable
```

**Interning Caveat:**

LOGOS may intern `Text` values for efficiency. Passing interned strings to C for modification would corrupt the intern table. The `Buffer` type prevents this by requiring explicit allocation.

### 10.7 Implementation Requirements

The standard library must be implemented in LOGOS (bootstrapped from Rust primitives) before the language is usable. FFI is an escape hatch, not a requirement.

**Phase 1: Core (Required for v1.0)**

| Module | Functions | Purpose |
|--------|-----------|---------|
| `logos:core` | Bool, Nat, Int, Real, Text | Foundation types |
| `logos:seq` | head, tail, append, map, filter, reduce | Data structures |
| `logos:map` | lookup, insert, remove, keys, values | Data structures |
| `logos:io` | Show, Read (console) | Basic I/O |
| `logos:text` | concat, split, contains, replace | String manipulation |

**Phase 2: Extended (Required for v1.1)**

| Module | Functions |
|--------|-----------|
| `logos:math` | sin, cos, sqrt, pow, log |
| `logos:time` | now, format, parse, duration |
| `logos:json` | parse, stringify |
| `logos:file` | read, write, exists, delete |

**Phase 3: Network (v2.0)**

| Module | Functions |
|--------|-----------|
| `logos:http` | get, post, request |
| `logos:tcp` | connect, listen, send, receive |

**Implementation Principle:**

Each standard library function should have:
1. An English signature (`To find the length of a sequence:`)
2. A proof of correctness (where applicable)
3. Zero FFI calls (except for OS primitives in `logos:io` and `logos:file`)

**Runtime Library Injection:**

The compiler automatically injects a preamble importing the runtime library into generated Rust code.

**Generated Rust Preamble:**
```rust
use logos_core::prelude::*;
```

This preamble provides:

| Export | Description |
|--------|-------------|
| `Nat` | `type Nat = u64;` |
| `Int` | `type Int = i64;` |
| `Text` | `type Text = String;` |
| `Real` | `type Real = f64;` |
| `Seq<T>` | `type Seq<T> = Vec<T>;` |
| `Map<K,V>` | `type Map<K,V> = HashMap<K,V>;` |

**The `logos_core` Crate:**

Before LOGOS programs can run, the `logos_core` crate must be available. This is a standard Rust crate that:
1. Re-exports Rust primitives with LOGOS type aliases
2. Provides trait implementations for LOGOS operations
3. Contains helper functions for generated code
4. **Wraps Rust macros as callable functions**

**Macro Wrappers:**

Since LOGOS FFI cannot call Rust macros directly (§10.6), `logos_core` provides function wrappers for essential macros:

| LOGOS Operation | logos_core Function | Underlying Macro |
|-----------------|---------------------|------------------|
| `Show x to the console.` | `logos_core::io::println(x)` | `println!("{}", x)` |
| `Log message.` | `logos_core::io::eprintln(msg)` | `eprintln!("{}", msg)` |
| `Panic with reason.` | `logos_core::panic_with(reason)` | `panic!("{}", reason)` |
| `Format "template [x]".` | `logos_core::fmt::format(...)` | `format!(...)` |

This ensures that standard library operations like `Show` and `Log` work without requiring users to write FFI wrappers.

**Bootstrapping Mechanism:**

The `logos_core` module is embedded directly into the compiler binary using Rust's `include_str!` macro. This ensures the compiler is a self-contained executable with no external dependencies for core functionality.

```rust
const LOGOS_CORE_SOURCE: &str = include_str!("../logos_core/src/lib.rs");
```

**Development Workflow:**
- The source is written in a standard `.rs` file for IDE support (syntax highlighting, type checking)
- At compiler build time, `include_str!` bakes the source into the binary
- During codegen, the compiler writes `logos_core` to `target/logos_core/` and references it as a path dependency

**Core Exports:**

| Symbol | Type | Description |
|--------|------|-------------|
| `Bool` | `type Bool = bool;` | Boolean truth value |
| `Nat` | `type Nat = u64;` | Natural number (non-negative) |
| `Int` | `type Int = i64;` | Signed integer |
| `Real` | `type Real = f64;` | Floating-point number |
| `Text` | `type Text = String;` | UTF-8 string |
| `Unit` | `type Unit = ();` | Empty type |
| `true`, `false` | `Bool` | Boolean constants |
| `nothing` | `Option<T>` | Empty optional |

**Dependency Resolution:**

The compiler generates `Cargo.toml` with a path dependency to the extracted `logos_core`:

```toml
[dependencies]
logos_core = { path = "./logos_core" }
```

---

## 11. Quality of Life

### 11.1 String Interpolation

Variables in brackets are interpolated:

```markdown
Log "User [name] is [age] years old.".
Show "Processing item [i] of [total]...".
Return "Error: [message] (code [code])".
```

Compiles to:
```rust
println!("User {} is {} years old.", name, age);
```

### 11.2 Magic Slices and Ranges

#### 11.2.1 The English Indexing Convention

LOGOS uses **1-based indexing** to match English intuition.

| English | LOGOS | Rust Codegen |
|---------|-------|--------------|
| "The 1st item of list" | `item 1 of list` | `list[0]` |
| "The 5th item of list" | `item 5 of list` | `list[4]` |
| "Items 2 through 5" | `items 2 through 5 of list` | `&list[1..5]` |

**Rationale:** In English, "the 1st item" never means index 0. LOGOS prioritizes natural language semantics over programmer convention.

**Safety Guard:**

The compiler rejects `item 0 of list` with an error:

```
Error at line 8: Indices start at 1.

  8 │ Let x be item 0 of list.
    │               ^
    │
  = In LOGOS, items are numbered starting from 1.
  = The first item is 'item 1', not 'item 0'.
```

#### 11.2.2 Pythonic Slicing

```markdown
Let top_scores be the first 5 items of scores.
Let last_three be the last 3 items of list.
Let middle be items 2 through 5 of sequence.
Let reversed be list in reverse order.
```

Compiles to (note the index translation):
```rust
let top_scores = &scores[..5];           // 1-indexed: first 5 = indices 0..5
let last_three = &list[list.len()-3..];  // last 3 = final 3 elements
let middle = &sequence[1..5];            // 1-indexed: 2 through 5 = indices 1..5
let reversed: Vec<_> = list.iter().rev().collect();
```

**Range Expressions:**
```markdown
Repeat for every n from 1 to 100:         // n = 1, 2, 3, ..., 100
Repeat for every n from 1 to length stepping by 2:  // n = 1, 3, 5, ...
Repeat for every character in text:
```

**Range Bounds:**

| LOGOS | Meaning | Rust |
|-------|---------|------|
| `from 1 to 100` | Inclusive both ends | `1..=100` |
| `from 1 up to 100` | Exclusive end | `1..100` |
| `from 1 to length` | 1 through length | `1..=len` |

### 11.3 The Socratic Compiler

Errors are conversations, not cryptic messages:

**Type mismatch:**
```
You wrote: Let result be x plus "hello".

I notice: You're trying to add an integer to text.
          Integers and text cannot be combined with 'plus'.

Perhaps you meant:
  1. Convert x to text first: "Let result be x as text combined with 'hello'."
  2. Parse the text as a number: "Let result be x plus 'hello' as an integer."
```

**Ownership error:**
```
You wrote: Show data to the console.

I notice: You gave ownership of 'data' to the processor on line 20.
          After giving something away, you cannot use it.

Perhaps you meant:
  1. Show data before giving it (move line 23 before line 20)
  2. Give a copy: "Give a copy of data to the processor"
  3. Only show it: "Show data to the processor" (borrow, not give)
```

**Proof failure:**
```
You wrote: Let result be x divided by y.

I notice: Division requires proof that y is not zero.
          I cannot verify this automatically.

Perhaps you meant:
  1. Add a precondition: "where y is not zero"
  2. Handle the zero case: "x divided by y, or 0 if y is zero"
  3. Prove it: "> y is not zero because [your reasoning]"
```

**Unused expression:**
```
You wrote: x plus 1.

I notice: This calculation produces a value, but you haven't used it.
          Expressions must be assigned, returned, or passed to a function.

Perhaps you meant:
  1. Assign the result: "Set y to x plus 1."
  2. Return the result: "Return x plus 1."
  3. Pass to a function: "Show x plus 1 to the console."
```

**Borrow conflict:**
```
You wrote:
    Let x be the first item of list.
    Remove the first item of list.

I notice: You are looking at 'x', which is inside the list (line 1).
          But then you tried to change the list (line 2) while still looking at 'x'.
          While someone is reading a value, no one else can modify it.

Perhaps you meant:
  1. Complete the read first, then modify:
     "Let x be the first item of list.
      Process x.
      Remove the first item of list."
  2. Make a copy before modifying:
     "Let x be a copy of the first item of list.
      Remove the first item of list."
```

### 11.4 Active Voice Enforcement (Style Linter)

The compiler encourages clear, direct prose:

```
Warning in "Data.md", line 15:
  You wrote: "The variable was set by the processor."

  Style: Passive voice makes code harder to follow.
         Who or what is doing the action?

  Suggestion: "The processor set the variable."
              or: "Set the variable via the processor."
```

```
Warning in "Math.md", line 8:
  You wrote: "It is computed that x equals y plus z."

  Style: "It is computed that" adds no meaning.

  Suggestion: "x equals y plus z."
```

---

## 12. Compilation Pipeline

### 12.1 Pipeline Overview

```
┌──────────┐    ┌────────┐    ┌───────────┐    ┌────────────┐
│ .md file │───▶│ Parser │───▶│ Type Check │───▶│ Proof Check │
└──────────┘    └────────┘    └───────────┘    └────────────┘
                                                      │
                    ┌─────────────────────────────────┴─────────────────────────────────┐
                    │                                                                   │
                    ▼                                                                   ▼
           ┌────────────────┐                                                  ┌────────────────┐
           │   Dev Mode     │                                                  │  Release Mode  │
           │  (Cranelift)   │                                                  │ (Rust → LLVM)  │
           └───────┬────────┘                                                  └───────┬────────┘
                   │                                                                   │
                   ▼                                                                   ▼
           ┌────────────────┐                                                  ┌────────────────┐
           │   JIT Binary   │                                                  │ Optimized Binary│
           │  (fast compile)│                                                  │  (fast runtime) │
           └────────────────┘                                                  └────────────────┘
```

#### 12.1.1 Imperative Determinism (The "No-Forest" Rule)

While the Declarative Mode (`## Theorem`) allows structural ambiguity (returning a Parse Forest), the Imperative Mode (`## Main`) enforces **Greedy Determinism**.

1. **Lexical Priority:** If a token is ambiguous (e.g., "Run" could be Noun or Verb):
   - Check the **Scope Stack**. If "Run" names a visible variable, it is a **Noun**.
   - If not, check if it names a function. If so, it is a **Verb**.
   - Default to **Noun** if unresolved, then error if invalid.

2. **Attachment Rule:** Prepositional Phrases (`with`, `in`) always attach to the **nearest** preceding noun or verb that accepts them (Right Association).

3. **No Forking:** The compiler disables `compile_forest` in imperative blocks. It commits to the first valid heuristic match or errors immediately.

### 12.2 Dual-Mode Compilation

LOGOS supports two compilation modes:

| Mode | Backend | Compile Speed | Runtime Speed | Use Case |
|------|---------|---------------|---------------|----------|
| **Dev** | Cranelift JIT | Fast (~100ms) | Moderate | Iteration, REPL, hot-reload |
| **Release** | Rust → LLVM | Slow (~10s) | Maximum | Production, benchmarks |

**Dev Mode features:**
- Instant feedback on changes
- Hot-reloading of proofs (change theorem, re-verify without full recompile)
- Debug assertions for proof obligations
- Source maps for English-to-error tracing

**Release Mode features:**
- Full LLVM optimization passes
- Proof erasure (zero overhead)
- Link-time optimization (LTO)
- Profile-guided optimization (PGO) support

**Command line:**
```bash
logos run program.md          # Dev mode (JIT)
logos build program.md        # Release mode
logos build --release program.md  # Explicit release
logos build --dev program.md      # Explicit dev
```

**Lazy Verification:**

In Dev Mode, complex SMT proof obligations are converted to runtime `debug_assert!` checks to maintain sub-second compilation speeds. Only simple syntactic constraints are verified statically. Full static verification occurs in Release Mode.

| Proof Type | Dev Mode | Release Mode |
|------------|----------|--------------|
| Simple bounds (`n > 0`) | Runtime assert | Static SMT |
| Complex refinements | Runtime assert | Static SMT |
| Termination proofs | Deferred | Static analysis |

### 12.3 Compilation Stages

1. **Markdown Parsing:** Extract structure (headers, bullets, blockquotes, code fences)
2. **Linguistic Parsing:** Parse English sentences to AST
3. **Type Inference:** Infer types, resolve overloading
4. **Type Checking:** Verify type correctness
5. **Lifecycle Analysis:** Track ownership and borrowing states
6. **Proof Checking:** Verify theorems and generated obligations
7. **Code Generation:** Emit Rust source code (release) or Cranelift IR (dev)
8. **Backend Compilation:** Invoke rustc (release) or Cranelift JIT (dev)

**Step 5: Lifecycle Analysis (Critical for Socratic Errors)**

To provide meaningful English error messages for ownership violations (§11.3), LOGOS implements its own simplified borrow checker rather than relying solely on `rustc` errors.

**Variable States:**

| State | Meaning |
|-------|---------|
| `Uninitialized` | Declared but not yet assigned |
| `Owned` | Variable holds ownership |
| `Moved` | Ownership transferred (via `Give`) |
| `Borrowed` | Immutable reference exists (via `Show`) |
| `MutBorrowed` | Mutable reference exists (via `Let modify`) |

**Analysis Output:**

For each variable at each program point, the analyzer tracks:
- Current ownership state
- Active borrows (with source locations)
- Zone membership (if allocated in a Zone)

**Socratic Error Generation:**

When a violation is detected:
```
Error: You gave 'data' to the processor (line 12),
       but then tried to show 'data' to the console (line 15).
       Once given, ownership cannot be reclaimed.
```

This phase runs *before* codegen, ensuring errors are reported in terms of English source, not Rust output.

**Lifecycle Analysis: The Hard Problem**

The ownership verbs (`Give`, `Show`, `Let modify`) require the compiler to track value lifetimes through all control flow paths. This is equivalent to implementing a simplified Rust borrow checker.

**State Transitions:**
```
Owned --Give--> Moved
Owned --Show--> Borrowed --> Owned (when borrow ends)
Owned --Let modify--> MutBorrowed --> Owned (when borrow ends)
```

**v0.1 Strategy (Defer to Rust):**

Full ownership analysis is complex. For the initial release, consider a hybrid approach:
1. Emit Rust code preserving LOGOS ownership semantics
2. Let `rustc` perform borrow checking
3. Translate Rust errors back to LOGOS source locations via source maps (see 12.6)

This defers the hardest verification work to a battle-tested implementation while maintaining correctness guarantees.

**Future Work:** Native LOGOS ownership analysis (Phase 5+) will enable richer Socratic errors that explain *why* ownership rules exist, not just that they were violated.

### 12.4 Error Messages (Socratic Style)

See Section 11.3 for detailed examples.

### 12.5 Incremental Compilation

The compiler caches:
- Parsed AST per module
- Type information
- Proof verification results

Only changed modules and their dependents recompile.

### 12.6 Source Mapping & Debug Information

Every AST node preserves its source location (`Span`). The compiler emits debug information mapping English sentences to executable instructions.

**Debug Formats:**

| Mode | Format | Contents |
|------|--------|----------|
| Dev (Cranelift) | Internal spans | Line/column → IR instruction |
| Release (LLVM) | DWARF | Line/column → machine address |

**Source Location Preservation:**

Proofs are erased at runtime, but their source locations are retained for error messages:

```markdown
Trust that x is positive because user input validated.
```

If this trust is violated (dev mode assertion), the error references line 42 of `Math.md`, not a generated Rust file.

**Stack Traces:**

Runtime failures produce both:
1. **The Story:** High-level narrative ("You asked to load config...")
2. **The Trace:** Low-level locations (`Math.md:42:5 → IO.md:18:3`)

**IDE Integration:**

The Live Codex displays inline annotations showing which sentences are currently executing during step-debugging. Breakpoints can be set on any sentence.

---

## 13. The Live Codex (IDE)

### 13.1 Vision: Code as Conversation

The Live Codex is an IDE where writing code feels like having a conversation with a brilliant assistant. The interface is split:

| Left Pane | Right Pane |
|-----------|------------|
| Markdown source | Logic Visualizer |
| English prose | Type information |
| Proof blocks | Verification status |

### 13.2 The Logic Visualizer

When you define a data structure, the IDE visualizes it:

```markdown
# A Binary Tree of [Items]

A BinaryTree is either:
    Empty.
    A Node with a value ([Item]), a left subtree, and a right subtree.
```

The right pane shows an interactive tree diagram that updates as you type.

**Function flow visualization:**
When you define a function, the IDE shows the data flow:
```
Input → [Step 1] → [Step 2] → [Step 3] → Output
           ↓           ↓
        [Branch A]  [Branch B]
```

### 13.3 Real-time Proof Status

Proof blocks are color-coded:

| Color | Status | Meaning |
|-------|--------|---------|
| 🟢 Green | Verified | Proof is complete and valid |
| 🟡 Yellow | In Progress | Proof is being checked |
| 🔴 Red | Failed | Proof has errors or gaps |
| ⚪ Gray | Deferred | Proof uses `Auto` (will be checked) |

The IDE shows proof trees on hover:
```
Theorem: n + 0 = n
├── Base case (n = 0): ✓ By definition
└── Inductive step (n = k + 1):
    ├── Hypothesis: k + 0 = k ✓
    ├── Goal: (k + 1) + 0 = k + 1
    └── By computation: ✓
```

### 13.4 The Teacher's Pass (Style Enforcement)

The IDE includes a real-time style checker based on Strunk & White principles:

**Active voice enforcement:**
```
⚠️ "The value was computed by the function"
   → "The function computed the value"
```

**Brevity suggestions:**
```
⚠️ "In order to calculate the result"
   → "To calculate the result"
```

**Clarity improvements:**
```
⚠️ "It is the case that x equals y"
   → "x equals y"
```

### 13.5 Hot-Reloading Proofs

In Dev Mode, proofs can be hot-reloaded:

1. You modify a theorem's proof
2. The IDE re-checks only that proof and its dependents
3. Green/red status updates instantly (typically <100ms)
4. The running program continues unaffected (proofs are erased)

This enables "proof-driven development" where you iterate on proofs as fast as you iterate on code.

### 13.6 Morphological Refactoring

The Live Codex understands English morphology. When renaming identifiers, it automatically handles grammatical transformations.

**Pluralization:**

| Original | Renamed | Transformation |
|----------|---------|----------------|
| `User` → `Account` | `Users` → `Accounts` | Regular plural |
| `Child` → `Kid` | `children` → `kids` | Irregular plural |
| `Mouse` → `Device` | `mice` → `devices` | Irregular → regular |

**Articles:**

| Original | Renamed | Transformation |
|----------|---------|----------------|
| `a User` | `an Account` | a → an (vowel) |
| `an Item` | `a Record` | an → a (consonant) |

**Possessives:**
- `the User's name` → `the Account's name`
- `its value` → `its value` (unchanged)

**Implementation:**

The IDE maintains a morphology engine that tracks:
1. The base form of every identifier
2. All inflected forms in the codebase
3. Grammatical context (article, possessive, plural)

Renaming propagates through all forms atomically.

---

## 14. Appendices

### 14.1 Grammar Summary (EBNF)

```ebnf
program        = module* ;
module         = header paragraph* section* ;
header         = "#"+ identifier ;
section        = "##"+ identifier paragraph* block* ;

block          = imperative_block | proof_block | code_fence ;
imperative_block = INDENT statement+ DEDENT ;
proof_block    = ">" theorem_decl "\n>" proof_body ;
code_fence     = "```" language "\n" raw_code "\n" "```" ;

statement      = simple_stmt "." | compound_stmt ;
simple_stmt    = let_stmt | set_stmt | return_stmt | call_stmt | invoke_stmt | stream_stmt | trust_stmt | assert_stmt | check_stmt ;
compound_stmt  = if_stmt | while_stmt | repeat_stmt | inspect_stmt | zone_stmt ;

let_stmt       = "Let" identifier "be" expression ;
set_stmt       = "Set" target "to" expression ;
if_stmt        = "If" condition "," consequence "."
               | "If" condition ":" block ("Otherwise:" block)? ;
return_stmt    = "Return" expression? ;
repeat_stmt    = "Repeat" "for" "every" identifier "in" expression ":" block ;
while_stmt     = "While" condition decreasing_clause? ":" block ;
decreasing_clause = "(" "decreasing" expression ")" ;
call_stmt      = "Call" function "with" arguments ;
inspect_stmt   = "Inspect" expression ":" match_arm+ ;
match_arm      = "If" "it" "is" pattern ("where" condition)? ("," action "." | ":" block) ;
zone_stmt      = "Inside" "a"? "new"? "zone" "called" string ":" block ;
stream_stmt    = "Pour" expression "into" identifier ;
trust_stmt     = "Trust" "that" proposition "because" justification ;
invoke_stmt    = function_verb arguments? ;
assert_stmt    = "Assert" "that" proposition ;
check_stmt     = "Check" "that" condition ;
try_expr       = "try" expression ;

expression     = term (binary_op term)* | try_expr ;
term           = literal | identifier | quantified | application | math_expr ;
math_expr      = "`" symbolic_math "`" ;
quantified     = quantifier identifier ("in" expression)? ("where" condition)? ;
quantifier     = "all" | "every" | "some" | "any" | "no" | "most" | "few" ;

generic_type   = identifier type_preposition type_param ("and" type_param)* ;
type_preposition = "of" | "from" | "to" ;
type_param     = "[" identifier "]" | identifier ;
type_annot     = identifier ":" type_expr ;

theorem_decl   = "**Theorem" name? ":**" proposition ;
proof_body     = "*Proof:*" proof_content ;
proof_content  = tactic | english_proof ;
tactic         = "Auto" | "By" justification ;

is_expr        = expression "is" is_rhs ;
is_rhs         = expression          (* Equality check *)
               | "a"? type_expr      (* Type/Variant check *)
               | adjective           (* Predicate application *)
               ;
```

**Function Invocation Resolution (`invoke_stmt`):**

The `function_verb` in `invoke_stmt` is resolved from the `GlobalSymbolTable`. During Pass 2, when the parser encounters a sentence starting with a verb, it first checks if that verb matches a function name registered during Pass 1. If matched, the sentence is parsed as a function invocation; otherwise, standard verb parsing applies.

**Boolean Operator Precedence:**

`And` binds tighter than `Or`. Mixed expressions without explicit grouping emit a lint warning.

| Expression | Interpretation | Warning? |
|------------|----------------|----------|
| `A and B or C` | `(A and B) or C` | Yes: "Consider explicit grouping" |
| `A or B and C` | `A or (B and C)` | Yes: "Consider explicit grouping" |
| `(A and B) or C` | `(A and B) or C` | No |
| `A or (B and C)` | `A or (B and C)` | No |

**Rationale:** English pauses ("A and B... or C") are ambiguous. The "Oxford Comma of Logic" requires explicit clarity when mixing boolean operators.

**The "is" Disambiguation Rule:**

The copula `is` is parsed based on the Right-Hand Side:

| Pattern | RHS Type | Interpretation | Example |
|---------|----------|----------------|---------|
| `x equals 5` | Expression | Equality (`x == 5`) | `If x equals 5` |
| `x is 5` | Literal | **ERROR** | Use `x equals 5` |
| `x is a User` | Type preceded by article | Type check | `If result is a Success` |
| `x is Success` | Enum variant name | Variant check | `If result is Success(data)` |
| `x is empty` | Adjective | Predicate application | `If list is empty` |
| `x is greater than y` | Comparative | Comparison | `If x is greater than y` |

See §4.1.3 for the full Equality Rule.

**Forbidden Constructs:**

| Pattern | Error |
|---------|-------|
| `Set x is 5` | Use `Set x to 5` |
| `Let x is 5` | Use `Let x be 5` |
| `x is 5` | Use `x equals 5` |

**Resolution Priority for `IS` in Imperative Mode:**

1. **Lookahead:** If `is` is followed by `a` or `an`, it is **always** a Type Check.
2. **Registry Check:** If the symbol following `is` exists in the `TypeRegistry` (as a Struct or Enum), it is a Type/Variant Check.
3. **Adjective Check:** If the symbol is in the `AdjectiveRegistry` (e.g., `empty`, `valid`), it is a Predicate Application.
4. **Error:** Otherwise, emit error and suggest `equals`.

**The "Empty" Disambiguation:**

The word `empty` can mean either "create an empty collection" or "test for emptiness." The distinction is syntactic:

| Syntax | Meaning | Example |
|--------|---------|---------|
| `an empty [Type]` | Constructor | `Let list be an empty Sequence of Integers.` |
| `is empty` | Predicate | `If list is empty:` |
| `be empty` | **Forbidden** | `Let list be empty.` (Error) |

**Error Message:**

If the parser encounters `Let x be empty.`, it should emit:

```
Error: Ambiguous "be empty" — did you mean:
  - "an empty Sequence of [Type]" (to create an empty collection), or
  - "an empty Option of [Type]" (to create an optional value)?
```

### 14.2 Type Inference Algorithm

LOGOS uses bidirectional type checking with constraint generation:

1. **Synthesis:** Infer type from term structure
2. **Checking:** Check term against expected type
3. **Unification:** Solve type constraints
4. **Refinement solving:** Dispatch to SMT solver (Z3)

### 14.3 Proof Checking Algorithm

The proof kernel is a minimal trusted core:

1. **Parse proof term:** Convert English proof to proof term AST
2. **Type check proof:** Verify proof term has type of theorem
3. **Normalize:** Reduce proof term to normal form
4. **Verify:** Check all axioms are valid

### 14.4 Comparison with Related Work

| System | Similarity | Difference |
|--------|------------|------------|
| **Lean 4** | Dependent types, tactics | LOGOS uses English syntax |
| **Idris 2** | Dependent types, totality | LOGOS has ownership, agents |
| **Rust** | Ownership, performance | LOGOS has proofs, English |
| **Zig** | Manual memory (arenas), comptime | LOGOS has proofs, English |
| **Inform 7** | English-like syntax | LOGOS is general-purpose |
| **Literate Haskell** | Markdown source | LOGOS prose IS code |

### 14.5 Future Extensions

1. **Effect System:** Track effects (IO, State, Exception) in types
2. **Linear Types:** Finer control than ownership (use exactly once)
3. **Quotient Types:** Types modulo equivalence relation
4. **Homotopy Type Theory:** Univalence axiom for advanced mathematics
5. **Self-Hosting:** LOGOS compiler written in LOGOS

### 14.6 Complete Examples

#### Example 1: User Management

```markdown
# User Management

## Definition

A User has:
    a username, which is Text.
    an email, which is Text.
    a login_count, which is a Nat.
    an is_active status, which is a Bool.

A ValidUser is a User where:
    the login_count is greater than 0.
    the is_active status is true.

## To Authenticate

To authenticate the user with the password:
    Let max_attempts be 3.
    If the user's login_count is greater than max_attempts:
        Log "Account locked for [user.username]".
        Return Error("Too many attempts").
    Otherwise:
        Let hash be the result of hashing the password.
        If hash equals the stored_hash:
            Set the user's is_active status to true.
            Return Success(user).
        Otherwise:
            Increase the user's login_count by 1.
            Return Error("Invalid credentials").
```

#### Example 2: Mathematics with Proofs

```markdown
# Mathematics.Arithmetic

## Theorem

> **Theorem Identity:** For every natural number n, n plus 0 equals n.
>
> *Proof:* By induction on n.
> - **Base:** When n is 0, 0 plus 0 equals 0 by definition.
> - **Step:** Assume k plus 0 equals k. Then (k + 1) plus 0 equals k + 1 by the inductive hypothesis.

## Definition

The factorial of (n: Nat) is:
    If n is 0, then 1.
    Otherwise, n times the factorial of n minus 1.
```

#### Example 3: Concurrency with Agents

```markdown
# Network Systems

## Definition

The CacheAgent accepts:
    StoreRequests, which have a key and a value.
    RetrieveRequests, which have a key.

## Main

To run the server:
    Spawn the CacheAgent.
    Repeat forever:
        Await the next request from the network.
        Give request to the CacheAgent.
        Simultaneously:
            Log "Request received".
            Update the metrics.
```

#### Example 4: Zone-Based Rendering

```markdown
# Graphics Engine

## To Render Frame

To render the scene:
    Let canvas be the main display.
    Inside a new zone called "ScratchSpace":
        Let particles be 5000 Particle objects.
        Repeat for every p in particles:
            Update p's position.
            Draw p onto canvas.
    Log "Frame render complete".
```

---

## 15. Implementation Mechanics (The Engine Room)

This section bridges the gap between LOGOS English prose and the compiler's internal behavior. It defines the mapping from AST to Runtime.

### 15.0 The Kernel Architecture

LOGOS employs a **Kernel Architecture** where the declarative logic engine (Logicaffeine) forms the core, wrapped by an imperative execution layer.

**The Relationship:**

| Layer | Role | Origin |
|-------|------|--------|
| **Logic Kernel** | Handles `## Theorem`, `## Logic`, proofs | Logicaffeine engine (preserved) |
| **Imperative Layer** | Handles `## Main`, `## To [Verb]`, state | New LOGOS additions |
| **Bridge** | `Assert { proposition: &LogicExpr }` | Links layers |

**The Preservation Guarantee:**

All existing Logicaffeine functionality continues to work unchanged. The `## Theorem` blocks, Neo-Davidsonian event semantics, parse forest ambiguity resolution, and logical inference remain the kernel's responsibility. LOGOS builds *around* this kernel, not *instead of* it.

#### 15.0.0 The Dual-AST Architecture

LOGOS has two distinct semantic modes that require separate AST representations:

| Mode | Context | AST | Semantics |
|------|---------|-----|-----------|
| **Declarative** | `## Theorem`, `## Logic` | `LogicExpr` | Truth propositions |
| **Imperative** | `## Main`, `## To [Verb]` | `Stmt` | State changes |

**Why Separation Matters:**

A sentence like "John runs" has fundamentally different meanings:
- In Logic mode: `∃e(Run(e) ∧ Agent(e, John))` — a proposition about truth
- In Code mode: `john.run();` — a command to execute

Attempting to unify these in a single AST leads to semantic confusion.

**LogicExpr (Declarative AST):**

```rust
pub enum LogicExpr<'a> {
    Predicate { name: Symbol, args: Vec<Term<'a>> },
    Quantifier { kind: QuantKind, var: Symbol, body: Box<LogicExpr<'a>> },
    Connective { kind: ConnKind, left: Box<LogicExpr<'a>>, right: Box<LogicExpr<'a>> },
    Modal { kind: ModalKind, body: Box<LogicExpr<'a>> },
    // ... propositions, not commands
}
```

Used for theorems, lemmas, proofs, and logical definitions.

**Stmt (Imperative AST):**

```rust
pub enum Stmt<'a> {
    Let { name: Symbol, value: Expr<'a>, mutable: bool },
    Set { target: Symbol, value: Expr<'a> },
    If { cond: Expr<'a>, then_block: Block<'a>, else_block: Option<Block<'a>> },
    Return { value: Option<Expr<'a>> },
    While { cond: Expr<'a>, body: Block<'a> },
    Call { function: Symbol, args: Vec<Expr<'a>> },
    // ... state changes, not propositions
}
```

Used for function bodies, main blocks, and agent handlers.

**Parser Dispatch:**

The parser examines the header to determine which AST to construct:

| Header Pattern | AST | Parser Path |
|----------------|-----|-------------|
| `## Theorem:` | `LogicExpr` | `parse_theorem()` |
| `## Lemma:` | `LogicExpr` | `parse_lemma()` |
| `## Logic` | `LogicExpr` | `parse_logic()` |
| `## To [Verb]` | `Stmt` | `parse_function()` |
| `## Main` | `Stmt` | `parse_main()` |
| `## Definition` | Either | Context-dependent |

#### 15.0.2 The Parser Mode Switch

To support both Logicaffeine (declarative) and LOGOS (imperative), the `Parser` struct maintains a `Mode` state.

```rust
enum ParserMode {
    Declarative, // Logicaffeine: Ambiguity allowed, auto-constants, NeoEvents
    Imperative,  // LOGOS: Deterministic, strict scoping, MethodCalls
}
```

**Behavioral Differences:**

| Feature | Declarative Mode | Imperative Mode |
|---------|------------------|-----------------|
| **"John runs"** | Parses to `NeoEvent(Run, Agent: John)` | Parses to `Call { obj: "John", method: "run" }` |
| **Unknown Word** | Auto-registers as Global Entity | Error: "Variable not found" |
| **Ambiguity** | Forks into Parse Forest | Uses Greedy Heuristic (Right-Assoc) |
| **Output AST** | `LogicExpr` (Propositions) | `Stmt` (Instructions) |

The shared `Lexer` remains valid, but the `ClauseParsing` and `VerbParsing` traits must dispatch to different logic based on this mode.

**Shared Subexpressions:**

Both ASTs share a common `Expr` type for pure expressions (arithmetic, function calls, field access) that don't imply truth or state:

```rust
pub enum Expr<'a> {
    Literal(Literal),
    Identifier(Symbol),
    BinaryOp { op: Op, left: Box<Expr<'a>>, right: Box<Expr<'a>> },
    Call { function: Symbol, args: Vec<Expr<'a>> },
    FieldAccess { object: Box<Expr<'a>>, field: Symbol },
    // ... pure computations
}
```

This `Expr` is embedded within both `LogicExpr` (as terms) and `Stmt` (as values).

**AST Hierarchy Diagram:**

```
                    ┌─────────────────┐
                    │  Expr (Shared)  │
                    │  - Literals     │
                    │  - Identifiers  │
                    │  - Binary ops   │
                    │  - Field access │
                    └────────┬────────┘
                             │
            ┌────────────────┴────────────────┐
            ▼                                 ▼
    ┌───────────────┐                 ┌───────────────┐
    │   LogicExpr   │                 │     Stmt      │
    │  (Declarative)│                 │  (Imperative) │
    ├───────────────┤                 ├───────────────┤
    │ Predicate     │                 │ Let           │
    │ Quantifier    │                 │ Set           │
    │ Connective    │                 │ If/While      │
    │ Modal         │                 │ Return        │
    │ NeoEvent      │                 │ MethodCall    │
    └───────────────┘                 └───────────────┘
```

**Transition from Current Codebase (The Kernel Split):**

The current Logicaffeine `Expr` enum contains 38+ variants mixing logical constructs and value expressions. The transition:

| Step | Action | Result |
|------|--------|--------|
| **Phase 0.1** | Rename `Expr` → `LogicExpr` globally | Clarifies purpose: propositions only |
| **Phase 0.2** | Move to `src/ast/logic.rs` | Module separation |
| **Phase 0.3** | Create new `Expr` for pure values | Shared by both ASTs |
| **Phase 0.4** | Create `Stmt` in `src/ast/stmt.rs` | Imperative constructs |
| **Phase 0.5** | Add `ParserMode` flag | Dispatch mechanism |

**The Critical Invariant:**

All existing tests must pass after Phase 0. The rename from `Expr` to `LogicExpr` is semantic clarification, not behavioral change. The Logic Kernel continues to function identically.

This separation ensures the parser cannot accidentally produce a `NeoEvent` in imperative context or a `Stmt` in a theorem.

#### 15.0.3 Verb Parsing Trait Split

To enforce the Dual-AST architecture, verb parsing uses **two separate traits** rather than a single trait with a union return type.

**LogicVerbParsing (Declarative Mode):**

```rust
pub trait LogicVerbParsing<'a> {
    fn parse_predicate_with_subject(&mut self, subject: Symbol) -> ParseResult<&'a LogicExpr<'a>>;
    fn parse_neo_event(&mut self, verb: Symbol) -> ParseResult<&'a NeoEventData<'a>>;
    // Returns LogicExpr (propositions, truth conditions)
}
```

**ImperativeVerbParsing (Imperative Mode):**

```rust
pub trait ImperativeVerbParsing<'a> {
    fn parse_method_call(&mut self, subject: Symbol) -> ParseResult<&'a Stmt<'a>>;
    fn parse_function_call(&mut self, verb: Symbol) -> ParseResult<&'a Stmt<'a>>;
    // Returns Stmt (instructions, state changes)
}
```

**Dispatch Logic:**

```rust
match self.mode {
    ParserMode::Declarative => self.parse_predicate_with_subject(subject),
    ParserMode::Imperative => self.parse_method_call(subject),
}
```

**Rationale:**

Separate traits prevent accidental mixing of semantics. A `NeoEvent` can never appear in imperative code; a `Stmt` can never appear in a theorem. This is enforced at compile time by the Rust type system.

**Shared Infrastructure:**

Both traits may share:
- `Lexer` (tokenization is mode-agnostic)
- `parse_arguments()` (argument list parsing)
- `lookup_verb()` (lexicon lookup)

But they must NOT share the final AST construction logic.

### 15.0.1 Type vs. Sort (Dual Classification)

LOGOS maintains two parallel classification systems for entities:

| System | Purpose | Example |
|--------|---------|---------|
| **Sort** (Ontology) | Semantic validity | `Human`, `Animate`, `Physical` |
| **Type** (TypeRegistry) | Compilation | `Int`, `Text`, `Struct { id: Int }` |

**Why Both Are Needed:**

A `User` in LOGOS can be both:
- Sort `Human` — for semantic analysis ("The user laughed" is valid; "The user evaporated" is metaphorical)
- Type `Struct { id: Int, name: Text }` — for code generation (Rust struct)

**Ontology (Sort System):**

Used by the semantic analyzer to:
- Detect metaphors and selectional violations
- Validate predicate-argument compatibility
- Track animacy, gender, number for pronouns

```rust
pub struct Ontology {
    sorts: HashMap<Symbol, Sort>,
    hierarchy: SortLattice,
}

pub enum Sort {
    Human,
    Animate,
    Physical,
    Abstract,
    // ...
}
```

**TypeRegistry (Type System):**

Used by the code generator to:
- Determine Rust types for bindings
- Validate function signatures
- Emit correct struct definitions

```rust
pub struct TypeRegistry {
    types: HashMap<Symbol, TypeDef>,
    generics: HashSet<Symbol>,
}

pub enum TypeDef {
    Primitive(PrimType),
    Struct { fields: Vec<(Symbol, TypeDef)> },
    Enum { variants: Vec<Variant> },
    Generic { params: Vec<Symbol>, body: Box<TypeDef> },
}
```

**Entity Resolution:**

When the parser encounters an entity like "the user", it queries both systems:

1. **Ontology query:** What Sort is `user`? → `Human` (for semantic analysis)
2. **TypeRegistry query:** What Type is `user`? → `Struct { id, name, email }` (for codegen)

**Example:**

```markdown
# A User

A User is a person.          ← Ontology: Sort = Human
A User has:                  ← TypeRegistry: Type = Struct
    an id (Nat).
    a name (Text).
```

Both classifications are derived from the same definition but serve different compiler phases.

### 15.1 The Adjective System (Generics)

**The Prose:**

```markdown
A Stack of [Things]
...
Let numbers be a Stack of Integers.
```

**The Mechanics:**

1. **Parser Detection:** When parsing a type definition, the parser identifies brackets `[...]` as **Type Parameters**. The preposition preceding it (usually `of`, `from`, or `to`) binds the parameter.

2. **AST Representation:**
```rust
enum Type {
    Generic {
        base: Symbol,
        params: Vec<Symbol>,
    },
}
```

3. **Monomorphization:** At compile time, LOGOS generates specialized copies of the code for every concrete usage.
    - `Stack of Integers` generates `struct Stack_Int`.
    - `Stack of Users` generates `struct Stack_User`.

### 15.2 The Zone System (Memory Arenas)

**The Prose:**

```markdown
Inside a new zone called "Scratch":
    Let particles be 5000 Particle objects.
```

**The Mechanics:**

1. **AST Node:**
```rust
enum Expr {
    Zone {
        label: Symbol,
        body: Box<Expr>,
    },
}
```

2. **Codegen (Rust):**
```rust
{
    let scratch_arena = bumpalo::Bump::new();
    {
        let particles = scratch_arena.alloc_slice_fill_default(5000);
    }
}
```

3. **Region Inference (Safety):** The type checker enforces the **Escape Rule**: Any reference derived from `scratch_arena` is tagged with a lifetime `'zone`. If the code attempts to return a `'zone` reference outside the block, compilation fails.

### 15.3 The Socratic Error System (Traceability)

**The Prose:**

```
**The Story:**
1. You asked to "Load the user configuration".
2. Which tried to "Read the contents of file".
```

**The Mechanics:**

1. **The Failure ABI:** All fallible functions implicitly return `Result<T, Failure>`.
```rust
struct Failure {
    message: String,
    story: Vec<&'static str>,
    state: HashMap<String, String>,
}
```

2. **Instrumentation:** The parser wraps every function call site with an error handler that appends context.
```rust
match io::read_file(path) {
    Ok(val) => val,
    Err(mut fail) => {
        fail.story.push("tried to Read file at path");
        return Err(fail);
    }
}
```

3. **State Capture (Dev Mode):** If compiled in `--dev` mode, the error handler serializes all local variables in scope into the `state` map before returning.

**Codegen Pattern:**

The compiler generates error-wrapping code at every fallible call site:

```rust
// LOGOS: Let result be read the file at path.
// Generated Rust:
let result = read_file(path).map_err(|mut e| {
    e.story.push("tried to 'Read the file at path'");
    e
})?;
```

**String Interning:**

Story entries are interned as `&'static str` during compilation. The English description is derived from the source line and embedded in the binary. This adds minimal binary size overhead (~100 bytes per call site).

**Release Mode:**

In `--release` builds, Story accumulation can be disabled via compiler flag `--no-story` to eliminate the `.map_err()` overhead entirely.

### 15.4 Agents & Wire Protocol

**The Prose:**

```markdown
Give the SetRequest to the CacheAgent.
```

**The Mechanics:**

1. **Enum Generation:** The compiler aggregates all "Accepts" clauses for an Agent into a Message Enum.
```rust
enum CacheAgentMessage {
    GetRequest { key: String, reply_to: Sender<Option<String>> },
    SetRequest { key: String, value: String, reply_to: Sender<()> },
}
```

2. **The "Give" Verb:**
    - **Local Agent:** Compiles to `tokio::sync::mpsc::Sender::send()`. Ownership is moved to the channel.
    - **Remote Agent:** Compiles to `serde_json::to_vec()` + TCP write.

3. **Ownership Check:** The compiler marks the variable `SetRequest` as **Moved** (unusable) immediately after the "Give" statement.

### 15.5 Totality Checking (Termination)

**The Prose:**

```markdown
decreasing x
```

**The Mechanics:**

1. **CFG Analysis:** The compiler builds a Control Flow Graph.

2. **Cycle Detection:** Every cycle (loop/recursion) must have a **Variant**.

3. **Variant Verification:**
    - The compiler attempts to verify `variant_next < variant_current` via a lightweight SMT check.
    - For `decreasing x` (where x is Nat), it verifies `x` reduces toward 0.
    - If verification fails, a **Totality Error** is raised at compile time.

### 15.6 Structured Concurrency

**The Prose:**

```markdown
Attempt all of the following:
    Task A.
    Task B.
```

**The Mechanics:**

1. **Tokio Join:** This constructs a localized `tokio::join!` (or `try_join!`) future.
```rust
let (res_a, res_b) = tokio::join!(
    async { /* Task A body */ },
    async { /* Task B body */ }
);
```

2. **Cancellation Safety:** Because `join!` polls futures together, if the parent scope is dropped (e.g., via a return statement), all child futures are dropped (cancelled) instantly.

### 15.7 Mixed Math Parsing

**The Prose:**

```markdown
Let y be `mx + b`.
Let distance be the square root of a squared plus b squared.
```

**The Mechanics:**

1. **Normalization:** Both inputs parse to the exact same AST nodes.
    - `mx + b` → `BinaryOp(Add, BinaryOp(Mul, m, x), b)`
    - "m times x plus b" → `BinaryOp(Add, BinaryOp(Mul, m, x), b)`

2. **Symbolic Parser:** The backtick triggers a standard Pratt Parser implementation within the Lexer, allowing users to switch between Prose and Math notation at will.

### 15.8 Refinement Types (Constraint Solving)

**The Prose:**

```markdown
A ValidUser is a User where login_count > 0.
```

**The Mechanics:**

1. **Verification Conditions (VC):** When an instance of `ValidUser` is created, the compiler generates a logical assertion: `assert(login_count > 0)`.

2. **SMT Solver:** This assertion is passed to an integrated solver (like Z3).
    - If the solver returns **UNSAT** (logic violation), a compile error occurs.
    - If the solver returns **SAT** (valid), the code compiles.

3. **Runtime Erasure:** In the final binary, `ValidUser` is identical to `User`. The check is purely compile-time (Zero Cost).

---

### Implementation Checklist for v0.3.0

To support these mechanics, the codebase requires:

- [ ] **Parser:** Update to handle indentation-based scoping (dedent/indent tokens).
- [ ] **Generics:** Implement "Adjective" detection in the type parser.
- [ ] **Runtime:** Implement the `Failure` struct and `Result` mapping.
- [ ] **Memory:** Add `bumpalo` integration for Zone blocks.
- [ ] **Analysis:** Integrate a basic SMT solver (or simple linear inequality checker) for refinement types and totality checking.

---

## 16. Implementation Roadmap (v0.5.2 - Kernel Architecture)

This roadmap guides the transition from the Logicaffeine prototype to the LOGOS v0.5 compiler using the **Kernel Architecture** pattern (see §15.0).

### Phase 0: The Kernel Split (PREREQUISITE)

**Goal:** Prepare the architecture for Dual-AST without breaking existing tests. Logicaffeine becomes the "Logic Kernel."

**Commit Strategy:** Each substep is a separate commit with `cargo test` verification:

| Step | Commit Message | Gate |
|------|----------------|------|
| 0.1 | `refactor: rename Expr to LogicExpr` | All tests pass |
| 0.2 | `refactor: move LogicExpr to src/ast/logic.rs` | All tests pass |
| 0.3 | `feat: add Stmt placeholder to src/ast/stmt.rs` | Compiles |
| 0.4 | `feat: add ParserMode enum to Parser` | All tests pass |
| 0.5 | `test: verify Logic Kernel unchanged` | All tests pass |

**Rollback:** If any step breaks tests, `git reset --hard HEAD~1` and investigate.

**Checklist:** ✅

- [x] **Step 0.1:** Rename `Expr` → `LogicExpr` globally (propositions only).
- [x] **Step 0.2:** Create `src/ast/logic.rs`, move `LogicExpr` and related types (`Quantifier`, `NeoEvent`, `Term`).
- [x] **Step 0.3:** Create `src/ast/stmt.rs` with placeholder `Stmt` enum containing `Assert { proposition: &LogicExpr }`.
- [x] **Step 0.4:** Add `mode: ParserMode { Logic, Imperative }` to `Parser` struct.
- [x] **Step 0.5:** Run full test suite; all existing Logicaffeine tests pass unchanged.

### Phase 1: The Architectural Split (Enhanced)

*Original goal preserved; enhanced with Council recommendations.*

- [ ] **Two-Pass Compilation:**
  - Pass 1: Discovery (TypeRegistry, GlobalSymbolTable, GenericSet)
  - Pass 2: Body parsing with Registry context
- [x] **NEW: Add `OwnershipState`:** Implement `enum OwnershipState { Owned, Moved, Borrowed }` in symbol table.
- [ ] **NEW: Use-After-Move Detection:** Track `Give` verb usage; emit immediate error on reuse.
- [ ] **Split Traits:** `LogicVerbParsing` vs `ImperativeVerbParsing` separation.
- [x] **Gate:** `## Theorem` blocks use Logic mode; `## Main` uses Imperative mode.

### Phase 2: Imperative Scope & Resolution (Enhanced) ✅

*Original goal preserved; enhanced with Council recommendations.*

- [x] **Implement `ScopeStack`:** Track variable bindings and function arguments.
- [x] **NEW: Add `equals` keyword:** Implement equality via `equals` instead of `is`.
- [x] **NEW: Reject `x is [value]`:** Emit error for value equality patterns, suggest `x equals [value]`.
- [x] **NEW: 1-Indexed Convention:** Implement `i-1` translation in index access codegen.
- [x] **NEW: Index 0 Guard:** Reject `item 0 of list` with Socratic error.
- [x] **Strict Resolution:** Check `ScopeStack` before `TypeRegistry`.
- [x] **Gate:** `x is 5` errors; `item 0 of list` errors.

### Phase 3: The Imperative AST (`Stmt`) ✅

*Original goal preserved.*

- [x] **Define `Stmt` Enum:** `Let`, `Set`, `Call`, `If`, `While`, `Return`, `Assert`.
- [x] **The Bridge:** `Stmt::Assert { proposition: &LogicExpr }` links to Logic Kernel.
- [x] **Block Scoping:** Use `LineLexer` INDENT/DEDENT tokens.
- [x] **Gate:** Simple imperative programs compile.

### Phase 4: Codegen Backend (Enhanced)

*Original goal preserved; enhanced with Council recommendations.*

- [x] **Create `src/codegen.rs`:** Emit Rust source from `Stmt` AST.
- [x] **NEW: 1-Indexed Slices:** `items 2 through 5` → `&list[1..5]` with bounds check.
- [x] **NEW: Boolean Precedence:** `And` binds tighter than `Or`.
- [ ] **Ownership Semantics:** `Give` → move, `Show` → `&`, `Let modify` → `&mut`.
- [ ] **Runtime Library:** `logos_core` crate with standard types.
- [x] **Gate:** "Hello World" compiles and runs via `cargo`.

### Phase 5: Verification Integration (Clarified)

*Original goal preserved; SMT integration deferred.*

- [x] **Assert Bridge:** `Stmt::Assert` invokes Logic Kernel for verification.
- [x] **CLARIFIED: v0.5 uses `debug_assert!`:** Refinements become runtime assertions.
- [ ] **DEFERRED: Z3/SMT to v0.6+:** Full static verification in release mode only.
- [ ] **Socratic Errors:** Handle `ScopeError`, `TypeError`, `OwnershipError`.
- [x] **Gate:** `Assert that x is greater than 0.` generates `debug_assert!(x > 0)`.

---

## Revision History

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0-draft | 2024 | Initial specification |
| 0.2.0-draft | 2024 | Added: Generics (Adjective System), Zone System, Channels, Inspection System, Quality of Life, Dual-Mode Compilation, Proof Irrelevance, Mixed Math, Live Codex IDE |
| 0.2.1-draft | 2024 | Added: Context Headers, Block Scoping Rules, Constraint Grammar, FFI Type Marshaling, Block-scoped Zones, Identifier Syntax, Complete Examples |
| 0.3.0-draft | 2024 | **The Narrative Edition**: Removed bullet syntax in favor of pure indentation scoping, Smart Totality inference with `decreasing` annotation, Full Socratic Error System (Story/State/Suggestion) |
| 0.3.1-draft | 2024 | Added Section 15: Implementation Mechanics (The Engine Room) — AST/codegen mappings for Generics, Zones, Errors, Agents, Totality, Concurrency, Math, and Refinements |
| 0.4.0-draft | 2025 | Added: Hypertext Import System (Abstract Rule, URI Schemes), Resource Embedding, Interface Implementation, Precedence Grouping, Trust Verb (manual verification override) |
| 0.4.1-draft | 2025 | **The Tooling Edition**: Added Canonical Phrasebook (§4.1.1), Dependency Lockfiles (§2.3.6), Standard Library Implementation Requirements (§10.7), Source Mapping & Debug Info (§12.6), Morphological Refactoring (§13.6) |
| 0.4.2-draft | 2025 | **The Refinement Pass**: Monotonic Operator Rule (§2.11), Multi-word Identifiers (§2.9.2), Sum Types (§3.3), Runtime Validation (§3.5.2), Generic Type Precedence (§3.6), Zone Containment Rule (§8.5), Give/Send distinction with Portable constraint (§9.5-9.6) |
| 0.4.3-draft | 2025 | **The Scope Edition**: Flow-sensitive mutability (§4.3), Identifier vs argument separation (§2.9.2), Namespace access syntax (§2.3.2), Sum type field matching (§3.3), Agent handles and lifecycle (§9.5), Zone escape semantics (§8.5), Interface proof obligations (§2.3.5) |
| 0.4.4-draft | 2025 | **The Architecture Edition**: Indentation-based lexing (§2.5.2), Comma-separated arguments (§2.9.2), Generic vs possessive disambiguation (§3.6), Module-boundary mutability exception (§4.3), FFI macro restrictions (§10.6), Runtime library injection (§10.7), Dual-AST architecture (§15.0), Type vs Sort distinction (§15.0.1) |
| 0.5.0-draft | 2025 | **The Transition Edition**: Parser Mode Switch (§15.0.2), Identifier Resolution Order (§4.3.1), Imperative Determinism (§12.1.1), Type Parsing Grammar (§3.6), Variable Lifting (§7.7), Implementation Roadmap (§16) |
| 0.5.1-draft | 2025 | **The Air-Tight Edition**: Function invocation syntax (§4.1.1, §14.1), If disambiguation by mode (§2.3.1), Assert/Check wrappers (§4.3.2), Bare expression prohibition (§4.1.1), Scope exclusivity for generics (§3.6), logos_core bootstrapping via include_str! (§10.7), C string FFI conversion (§10.6.1), Socratic errors for unused expressions and borrow conflicts (§11.3) |
| 0.5.2-draft | 2025 | **The Council Edition**: Kernel Architecture (§15.0), 1-indexed arrays (§11.2.1), `is` prohibition for equality (§4.1.3), Boolean precedence (§14.1), Ownership stub (§8.1.1), SMT integration phasing (§3.5), Merged implementation roadmap with Phase 0 commit strategy (§16), LineLexer/WordLexer interface signatures (§2.5.2) |
| 0.5.3-impl | 2025 | **Implementation Update**: Marked completed items in §16 Roadmap. Phase 0 ✅, Phase 2 ✅, Phase 3 ✅. Phase 4: 1-Indexed Slices ✅, Boolean Precedence ✅. Phase 5: Assert Bridge ✅, debug_assert! ✅. All 6 gates met. 828 tests passing. |

---

*"In the beginning was the Word, and the Word was with Logic, and the Word was Code."*
