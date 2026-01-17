# RUSTDOC_PLAN.md

Comprehensive rustdoc documentation strategy for the logicaffeine workspace.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Cargo.toml Configuration](#2-cargotoml-configuration)
3. [Crate-Level Documentation Standards](#3-crate-level-documentation-standards)
4. [Module-Level Documentation Standards](#4-module-level-documentation-standards)
5. [Item-Level Documentation Standards](#5-item-level-documentation-standards)
6. [Code Examples Strategy](#6-code-examples-strategy)
7. [Cross-Linking Strategy](#7-cross-linking-strategy)
8. [Conditional Documentation](#8-conditional-documentation)
9. [Building and Hosting](#9-building-and-hosting)
10. [Documentation Testing](#10-documentation-testing)
11. [Implementation Priority](#11-implementation-priority)
12. [Style Guide](#12-style-guide)
13. [Documentation Quadrant Model](#13-documentation-quadrant-model-diátaxis-framework)
14. [Audience-Aware Writing Guidelines](#14-audience-aware-writing-guidelines)

**Appendices:**
- [Appendix A: Quick Reference Card](#appendix-a-quick-reference-card)
- [Appendix B: Example Crate Header](#appendix-b-example-crate-header)
- [Appendix C: Executable Agent Task Specifications](#appendix-c-executable-agent-task-specifications)

---

## 1. Executive Summary

### 1.1 Current State Analysis (Accurate Audit)

| Crate | Crate Docs | README | Module Docs | Item Docs | Overall |
|-------|-----------|--------|-------------|-----------|---------|
| `logicaffeine-base` | Excellent | Excellent (175 lines) | Good | Good | 90% |
| `logicaffeine-lexicon` | Good | Excellent (220 lines) | Fair | **CRITICAL GAPS** | 70% |
| `logicaffeine-kernel` | Excellent | Excellent (173 lines) | Good | Very Good | 85% |
| `logicaffeine-data` | Good | Excellent | Good | Partial | 85% |
| `logicaffeine-system` | Good | Excellent (9KB) | Partial | Partial | 75% |
| `logicaffeine-language` | Excellent (125 lines) | Outstanding (429 lines) | Mixed | Mixed | 80% |
| `logicaffeine-proof` | **Excellent (577 lines)** | Outstanding (278 lines) | Good | Excellent | **95%** |
| `logicaffeine-compile` | Good (91 lines) | Excellent (164 lines) | Sparse | **CRITICAL GAPS** | 65% |
| `logicaffeine-verify` | Excellent | Excellent (9.5KB) | Partial | Good | 85% |
| `logicaffeine-tests` | N/A | N/A | N/A | N/A | Test crate |

**Correction Notice:** A previous version of this document incorrectly stated that `logicaffeine_proof` was missing crate docs. This was wrong. The proof crate has **577 lines** of excellent documentation—it is the **best documented crate** in the workspace. The initial `//` comments should technically be `//!` for rustdoc, but the documentation quality is exceptional.

**Summary:**
- All 9 crates have crate-level documentation (proof has the most comprehensive)
- Existing workspace `rustdoc-args = ["--document-private-items"]`
- Named invariants documented (LAMPORT, MILNER, LISKOV, TARSKI)
- READMEs are authoritative and outstanding
- **Critical gaps:** `logicaffeine_lexicon` Feature enum (27 undocumented variants), `logicaffeine_compile` modules

### 1.2 Critical Gaps Identified

**Tier 1 - Critical (Missing core documentation):**

1. **`logicaffeine_lexicon`**: 27 `Feature` enum variants have NO doc comments
2. **`logicaffeine_lexicon`**: Struct fields (`VerbEntry`, `VerbMetadata`, etc.) lack documentation
3. **`logicaffeine_compile`**: `ui_bridge`, `analysis/*`, `codegen`, `interpreter`, `extraction` modules undocumented
4. **`logicaffeine_system`**: Native modules (`time`, `env`, `random`) have no doc comments
5. **`logicaffeine_verify`**: `LicenseValidator` and `LicensePlan` lack doc comments

**Tier 2 - Important (Incomplete):**

1. **`logicaffeine_data`**: Sequence CRDTs (`RGA`, `YATA`) lack README examples
2. **`logicaffeine_data`**: Type aliases (`Nat`, `Int`, etc.) lack inline doc comments
3. **`logicaffeine_kernel`**: Decision procedures (ring, lia, omega, cc, simp) minimal docs
4. **`logicaffeine_language`**: Individual module files lack comprehensive module-level docs

**Tier 3 - Enhancement (Decision guidance):**

1. "When to use which CRDT?" decision matrix missing
2. "Persistent vs Distributed vs Synced" feature selection guide missing
3. Counter-example generation strategy in verify not explained

### 1.3 Good Patterns to Preserve

These patterns are working well and should be maintained:

1. **Architectural invariants upfront** (LAMPORT, MILNER, LISKOV, TARSKI)
2. **README as authoritative source** with tables and examples
3. **Re-export tables at lib.rs** showing public API
4. **Formal typing rules in doc comments** (kernel/type_checker.rs)
5. **Feature matrices with size estimates** (system)
6. **Platform support matrices** (system, data)

### 1.4 Goals

Achieve documentation quality comparable to exemplary Rust crates:
- **serde**: Comprehensive examples, clear type hierarchies
- **tokio**: Feature flag documentation, architecture guides
- **clap**: Derive macro examples, migration guides

Target metrics:
- 100% public items documented
- Every public type has at least one runnable example
- All feature flags documented with `#[doc(cfg(...))]`
- All cross-crate dependencies linked with intra-doc links

---

## 2. Cargo.toml Configuration

### 2.1 Workspace-Level Metadata

Add to `/Cargo.toml` (workspace root):

```toml
[workspace.package]
version = "0.6.0"
authors = ["Tristen Harr <tristen@brahmastra-labs.com>"]
edition = "2021"
license = "BUSL-1.1"
repository = "https://github.com/Brahmastra-Labs/logicaffeine"
homepage = "https://logicaffeine.com"
documentation = "https://docs.logicaffeine.com"
keywords = ["logic", "fol", "nlp", "theorem-prover", "crdt"]
categories = ["science", "parser-implementations", "text-processing"]

[workspace.metadata.docs.rs]
all-features = false
features = ["cli", "verification"]
cargo-args = ["--workspace"]
rustdoc-args = [
    "--document-private-items",
    "--enable-index-page",
    "-Zunstable-options"
]
default-target = "x86_64-unknown-linux-gnu"
targets = ["x86_64-unknown-linux-gnu", "wasm32-unknown-unknown"]
```

### 2.2 Per-Crate Cargo.toml Template

Each crate's Cargo.toml should inherit workspace metadata and add crate-specific fields:

```toml
[package]
name = "logicaffeine-{name}"
description = "{one-line description}"
version.workspace = true
authors.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true
keywords = ["{crate-specific}", "keywords"]
categories = ["{crate-specific}", "categories"]

[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

### 2.3 Crate-Specific Keywords and Categories

| Crate | Keywords | Categories |
|-------|----------|------------|
| `base` | `["arena", "interner", "span", "allocation"]` | `["memory-management", "data-structures"]` |
| `lexicon` | `["lexicon", "nlp", "vendler", "aktionsart"]` | `["parser-implementations", "text-processing"]` |
| `kernel` | `["type-theory", "coc", "calculus-constructions"]` | `["science", "mathematics"]` |
| `data` | `["crdt", "wasm", "distributed", "lamport"]` | `["data-structures", "wasm"]` |
| `system` | `["io", "networking", "persistence"]` | `["os", "filesystem", "network-programming"]` |
| `language` | `["fol", "nlp", "parser", "transpiler"]` | `["parser-implementations", "text-processing"]` |
| `proof` | `["theorem-prover", "backward-chaining", "socratic"]` | `["science", "mathematics"]` |
| `compile` | `["compiler", "codegen", "interpreter"]` | `["compilers", "development-tools"]` |
| `verify` | `["z3", "smt", "verification", "static-analysis"]` | `["development-tools", "science"]` |

---

## 3. Crate-Level Documentation Standards

### 3.1 Template for `//!` Crate Docs

Every `lib.rs` should begin with comprehensive crate documentation:

```rust
//! # {Crate Name}
//!
//! {One-paragraph description of what the crate does and its role in the workspace.}
//!
//! ## Quick Start
//!
//! ```rust
//! use logicaffeine_{crate}::{MainType};
//!
//! // Minimal working example (3-5 lines)
//! let example = MainType::new();
//! ```
//!
//! ## Feature Flags
//!
//! | Flag | Description | Default |
//! |------|-------------|---------|
//! | `flag-name` | What it enables | No |
//!
//! ## Architecture
//!
//! {Brief description of the crate's internal architecture, key abstractions.}
//!
//! ```text
//! ┌─────────┐     ┌─────────┐     ┌─────────┐
//! │ Input   │────▶│ Process │────▶│ Output  │
//! └─────────┘     └─────────┘     └─────────┘
//! ```
//!
//! ## Named Invariants
//!
//! - **{INVARIANT_NAME}**: {Description of the invariant and why it matters.}
//!
//! ## See Also
//!
//! - [`logicaffeine_other`]: {Relationship to this crate}
//! - [LOGOS Documentation](https://docs.logicaffeine.com)
```

### 3.2 Action Items by Crate

#### `logicaffeine-base` (5 files)
- [x] Crate-level docs present
- [ ] Add Quick Start section
- [ ] Add Architecture diagram (Arena → Interner → Span relationship)
- [ ] Document zero-copy philosophy

#### `logicaffeine-lexicon` (3 files) **CRITICAL GAPS**
- [x] Crate-level docs present
- [x] README excellent (220 lines)
- [ ] **CRITICAL: Document all 27 Feature enum variants (types.rs)**
- [ ] **CRITICAL: Add field docs to VerbEntry, VerbMetadata, NounMetadata, AdjectiveMetadata**
- [ ] Document VerbClass methods with Vendler theory references
- [ ] Add Sort hierarchy documentation in code (not just README)
- [ ] Add Quick Start with VerbClass example

#### `logicaffeine-kernel` (21 files)
- [x] Crate-level docs present
- [x] MILNER INVARIANT documented
- [ ] Add Quick Start with Term construction
- [ ] Add Architecture diagram (Term → Context → TypeChecker flow)
- [ ] Document CIC hierarchy (Prop : Type 0 : Type 1 : ...)

#### `logicaffeine-data` (21 files)
- [x] Crate-level docs present
- [x] LAMPORT INVARIANT documented
- [ ] Add Quick Start with ORSet example
- [ ] Add Architecture diagram (CRDT lattice hierarchy)
- [ ] Document WASM safety guarantees

#### `logicaffeine-system` (22 files)
- [x] Crate-level docs present (Cerf/Drasner Amendment)
- [ ] Add Quick Start for each feature flag
- [ ] Add feature matrix table
- [ ] Document platform compatibility (native vs WASM)

#### `logicaffeine-language` (47 files)
- [x] Crate-level docs present
- [ ] Add Quick Start with `compile()` example
- [ ] Add Architecture diagram (Lexer → Parser → AST → Transpiler)
- [ ] Document linguistic phenomena coverage

#### `logicaffeine-proof` (7 files) **BEST DOCUMENTED - 95%**
- [x] **Excellent crate-level docs (577 lines)** - Best in workspace
- [x] LISKOV INVARIANT documented
- [x] Comprehensive architecture explanation
- [x] Backward chaining strategy documented
- [x] Socratic hint generation documented
- [ ] **Minor:** Convert leading `//` comments to `//!` for rustdoc (style fix only)
- [ ] Add more inline code examples in Quick Start

**Note:** This crate was previously incorrectly assessed as "critical." The proof crate actually has the most comprehensive documentation in the entire workspace with detailed explanations of:
- Proof search architecture
- Goal-driven backward chaining
- Socratic tutoring integration
- Curry-Howard correspondence

The only technical issue is that some leading comments use `//` instead of `//!`, which affects rustdoc rendering but not documentation quality.

#### `logicaffeine-compile` (17 files) **CRITICAL GAPS**
- [x] Crate-level docs present (91 lines)
- [x] README excellent (164 lines)
- [ ] **CRITICAL: Expand lib.rs crate-level docs from 91 to ~300 lines**
- [ ] **CRITICAL: Document ui_bridge.rs** (high-level compilation API, 35KB)
- [ ] **CRITICAL: Document analysis/escape.rs** (EscapeChecker, EscapeError)
- [ ] **CRITICAL: Document analysis/ownership.rs** (OwnershipChecker, VarState)
- [ ] **CRITICAL: Document codegen.rs** (code generation, 113KB)
- [ ] **CRITICAL: Document interpreter.rs** (tree-walking interpreter, 70KB)
- [ ] **CRITICAL: Document extraction/** (proof term extraction, 4 files)
- [ ] Add Quick Start with compilation example
- [ ] Add Architecture diagram (AST → Analysis → CodeGen)
- [ ] Document verification pass (when feature enabled)

#### `logicaffeine-verify` (19 files)
- [x] Crate-level docs present (excellent)
- [x] README excellent (9.5KB)
- [ ] **Document LicenseValidator and LicensePlan** (license.rs lacks doc comments)
- [ ] Document cache file format and 24-hour TTL
- [ ] Add Quick Start with verification example
- [ ] Document Smart Full Mapping strategy
- [ ] Add license requirement callout

---

## 4. Module-Level Documentation Standards

### 4.1 Template for Module `//!` Docs

Every module file should begin with:

```rust
//! # {Module Name}
//!
//! {One-paragraph description of the module's purpose.}
//!
//! ## Overview
//!
//! {2-3 sentences on the key types and their relationships.}
//!
//! ## Example
//!
//! ```rust
//! use logicaffeine_{crate}::{module}::{Type};
//!
//! // Minimal example
//! ```
//!
//! ## See Also
//!
//! - [`related_module`]
```

### 4.2 Module Inventory by Crate

Total modules needing documentation: **~110 modules** (excluding tests)

#### `logicaffeine-base` (4 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `arena` | Needs docs | High |
| `intern` | Needs docs | High |
| `span` | Needs docs | High |
| `error` | Needs docs | High |

#### `logicaffeine-lexicon` (2 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `types` | Needs docs | High |
| `runtime` | Needs docs | Medium |

#### `logicaffeine-kernel` (12 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `term` | Needs docs | Critical |
| `context` | Needs docs | Critical |
| `type_checker` | Needs docs | Critical |
| `reduction` | Needs docs | High |
| `interface` | Partial | High |
| `prelude` | Needs docs | Medium |
| `error` | Needs docs | Medium |
| `cc` | Needs docs | Medium |
| `lia` | Needs docs | Medium |
| `ring` | Needs docs | Medium |
| `omega` | Needs docs | Medium |
| `simp` | Needs docs | Medium |
| `positivity` | Needs docs | Medium |
| `termination` | Needs docs | Medium |

#### `logicaffeine-data` (3 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `crdt` | Needs docs | Critical |
| `types` | Needs docs | High |
| `indexing` | Needs docs | Medium |

#### `logicaffeine-system` (10+ modules)
| Module | Status | Priority |
|--------|--------|----------|
| `io` | Needs docs | Critical |
| `time` | Needs docs | High |
| `env` | Needs docs | Medium |
| `random` | Needs docs | Medium |
| `file` | Needs docs | Medium |
| `fs` | Needs docs | Medium |
| `storage` | Needs docs | Medium |
| `network` | Needs docs | Medium |
| `concurrency` | Needs docs | Medium |
| `distributed` | Needs docs | Medium |
| `crdt` (sync wrapper) | Needs docs | Medium |

#### `logicaffeine-language` (~30 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `lexer` | Needs docs | Critical |
| `parser` | Needs docs | Critical |
| `ast` | Needs docs | Critical |
| `token` | Needs docs | Critical |
| `compile` | Needs docs | Critical |
| `transpile` | Needs docs | High |
| `semantics` | Needs docs | High |
| `lambda` | Needs docs | High |
| `drs` | Needs docs | High |
| `error` | Needs docs | High |
| `lexicon` | Needs docs | Medium |
| `analysis` | Needs docs | Medium |
| `arena_ctx` | Needs docs | Medium |
| `formatter` | Needs docs | Medium |
| `mwe` | Needs docs | Medium |
| `ontology` | Needs docs | Medium |
| `pragmatics` | Needs docs | Medium |
| `registry` | Needs docs | Medium |
| `scope` | Needs docs | Medium |
| `session` | Needs docs | Medium |
| `suggest` | Needs docs | Medium |
| `symbol_dict` | Needs docs | Medium |
| `view` | Needs docs | Medium |
| `visitor` | Needs docs | Medium |
| `debug` | Needs docs | Low |
| `style` | Needs docs | Low |
| `proof_convert` | Needs docs | Medium |

#### `logicaffeine-proof` (6 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `engine` | Needs docs | Critical |
| `hints` | Needs docs | Critical |
| `unify` | Needs docs | High |
| `certifier` | Needs docs | High |
| `error` | Needs docs | Medium |
| `oracle` | Needs docs | Medium |

#### `logicaffeine-compile` (9 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `compile` | Needs docs | Critical |
| `ui_bridge` | Needs docs | Critical |
| `loader` | Needs docs | High |
| `analysis` | Needs docs | High |
| `codegen` | Needs docs | High |
| `diagnostic` | Needs docs | Medium |
| `sourcemap` | Needs docs | Medium |
| `extraction` | Needs docs | Medium |
| `interpreter` | Needs docs | Medium |
| `verification` | Needs docs | Medium |

#### `logicaffeine-verify` (4 modules)
| Module | Status | Priority |
|--------|--------|----------|
| `solver` | Needs docs | Critical |
| `ir` | Needs docs | High |
| `license` | Needs docs | Medium |
| `error` | Needs docs | Medium |

### 4.3 Executable Checklists with File References

These checklists provide specific file paths and agent-executable instructions for addressing critical documentation gaps.

#### Checklist: `logicaffeine_lexicon` (CRITICAL)

**File: `crates/logicaffeine_lexicon/src/types.rs`**

The `Feature` enum has 27 variants with NO doc comments. Each variant needs documentation explaining its linguistic meaning and example usage.

```
AGENT TASK: Add doc comment to EACH Feature variant explaining:
  - Linguistic meaning
  - Example words/sentences where this feature applies

Variants needing docs:
  [ ] Transitive - verb requires direct object (e.g., "eat", "love")
  [ ] Intransitive - verb takes no object (e.g., "sleep", "arrive")
  [ ] Ditransitive - verb takes two objects (e.g., "give X to Y")
  [ ] SubjectControl - infinitive subject = matrix subject (e.g., "want to VP")
  [ ] ObjectControl - infinitive subject = matrix object (e.g., "persuade X to VP")
  [ ] Raising - no theta-role assignment (e.g., "seem to VP")
  [ ] Opaque - blocks entailments (e.g., "seek", "want")
  [ ] Factive - presupposes complement truth (e.g., "know", "regret")
  [ ] Performative - utterance performs action (e.g., "promise", "bet")
  [ ] Collective - requires plural subject semantically (e.g., "gather", "disperse")
  [ ] Mixed - allows collective or distributive readings
  [ ] Distributive - distributes over individuals
  [ ] Weather - impersonal subject (e.g., "rain", "snow")
  [ ] Unaccusative - subject is underlying object (e.g., "arrive", "fall")
  [ ] IntensionalPredicate - creates intensional context
  [ ] Count - countable noun (e.g., "chair", "idea")
  [ ] Mass - non-countable noun (e.g., "water", "furniture")
  [ ] Proper - proper name
  [ ] Masculine - grammatically masculine
  [ ] Feminine - grammatically feminine
  [ ] Neuter - grammatically neuter
  [ ] Animate - living entity
  [ ] Inanimate - non-living entity
  [ ] Intersective - adjective intersects with noun (e.g., "red ball")
  [ ] NonIntersective - adjective doesn't intersect (e.g., "alleged thief")
  [ ] Subsective - adjective is subset (e.g., "skillful surgeon")
  [ ] Gradable - admits degree modification (e.g., "very tall")
  [ ] EventModifier - modifies events (e.g., "quickly", "carefully")

Example fix for Feature::Transitive:
  /// A verb that requires a direct object.
  ///
  /// Transitive verbs denote actions that pass from an agent to a patient.
  /// Examples: "eat" (eat *something*), "love" (love *someone*), "build" (build *something*).
  Transitive,
```

**Struct field docs needed in `types.rs`:**
```
[ ] VerbEntry: Fields lemma, time, aspect, class need docs
[ ] VerbMetadata: Fields lemma, class, time, aspect, features need docs
[ ] NounMetadata: Fields lemma, number, features need docs
[ ] AdjectiveMetadata: Fields lemma, features need docs
[ ] CanonicalMapping: Fields lemma, polarity need docs
[ ] MorphologicalRule: Fields suffix, produces need docs
```

#### Checklist: `logicaffeine_compile` (CRITICAL)

**File: `crates/logicaffeine_compile/src/ui_bridge.rs` (~800 lines)**
```
AGENT TASK: Add module-level //! docs explaining:
  - This is the primary UI compilation API
  - Key functions: compile_for_ui(), compile_for_proof(), compile_theorem_for_ui()
  - Key functions: verify_theorem(), interpret_for_ui()
  - Key types: CompileResult, ProofCompileResult, TheoremCompileResult
  - Key types: AstNode, TokenInfo, TokenCategory
```

**File: `crates/logicaffeine_compile/src/analysis/escape.rs` (~300 lines)**
```
AGENT TASK: Add module-level //! docs explaining:
  - Purpose of escape analysis in LOGOS
  - EscapeChecker struct: what it tracks
  - EscapeError enum: each variant's meaning
```

**File: `crates/logicaffeine_compile/src/analysis/ownership.rs` (~400 lines)**
```
AGENT TASK: Add module-level //! docs explaining:
  - Ownership tracking for LOGOS (linear/affine types)
  - OwnershipChecker struct
  - OwnershipError enum
  - VarState enum (Owned, Moved, Borrowed, etc.)
```

**File: `crates/logicaffeine_compile/src/codegen.rs` (~2500 lines)**
```
AGENT TASK: Add module-level //! docs explaining:
  - Code generation pipeline
  - Target language(s)
  - Key transformation passes
```

**File: `crates/logicaffeine_compile/src/interpreter.rs` (~1500 lines)**
```
AGENT TASK: Add module-level //! docs explaining:
  - Tree-walking interpreter architecture
  - Value representation
  - Environment/scope handling
```

**File: `crates/logicaffeine_compile/src/extraction/mod.rs`**
```
AGENT TASK: Add module-level //! docs explaining:
  - Purpose of proof term extraction
  - Curry-Howard correspondence in this context
  - Relationship to kernel types
```

#### Checklist: `logicaffeine_system` (Native Modules)

**File: `crates/logicaffeine_system/src/time.rs`**
```
AGENT TASK: Add doc comments to:
  - now() function
  - Any Timestamp types
  - Platform-specific behavior notes
```

**File: `crates/logicaffeine_system/src/env.rs`**
```
AGENT TASK: Add doc comments to:
  - Environment variable access functions
  - Security considerations
```

**File: `crates/logicaffeine_system/src/random.rs`**
```
AGENT TASK: Add doc comments to:
  - Random number generation functions
  - WASM compatibility notes
  - CSPRNG vs non-cryptographic notes
```

#### Checklist: `logicaffeine_verify` (License Module)

**File: `crates/logicaffeine_verify/src/license.rs`**
```
AGENT TASK: Add doc comments to:
  - LicenseValidator struct (purpose, usage)
  - LicensePlan enum (each variant: Free, Pro, Premium, Lifetime, Enterprise)
  - validate() method (what it checks, return values)
  - Cache file format documentation
  - 24-hour TTL documentation
```

---

## 5. Item-Level Documentation Standards

### 5.1 Struct Documentation Template

```rust
/// A {one-line description}.
///
/// {More detailed description if needed. Explain the purpose, common use cases,
/// and any important invariants.}
///
/// # Examples
///
/// ```rust
/// use logicaffeine_{crate}::{Type};
///
/// let instance = Type::new(args);
/// assert!(instance.is_valid());
/// ```
///
/// # See Also
///
/// - [`RelatedType`]: {relationship}
pub struct MyStruct {
    /// {Field description, including valid ranges or invariants.}
    pub field: FieldType,
}
```

### 5.2 Enum Documentation Template

```rust
/// {One-line description of what this enum represents.}
///
/// # Variants
///
/// {Brief overview of the variant categories if applicable.}
///
/// # Examples
///
/// ```rust
/// use logicaffeine_{crate}::{Enum};
///
/// let variant = Enum::VariantA;
/// match variant {
///     Enum::VariantA => { /* ... */ }
///     Enum::VariantB(x) => { /* ... */ }
/// }
/// ```
pub enum MyEnum {
    /// {Variant description, when it's used.}
    VariantA,

    /// {Variant description.}
    ///
    /// Contains `{type}` representing {meaning}.
    VariantB(Type),
}
```

### 5.3 Function Documentation Template

```rust
/// {One-line description in imperative mood: "Computes the...", "Returns the..."}
///
/// {Longer description if needed. Explain algorithm, complexity, edge cases.}
///
/// # Arguments
///
/// * `arg1` - {Description of the argument}
/// * `arg2` - {Description, including valid ranges}
///
/// # Returns
///
/// {Description of the return value. For `Result`, describe both Ok and Err.}
///
/// # Errors
///
/// Returns [`ErrorType::Variant`] if {condition}.
///
/// # Panics
///
/// Panics if {condition}. (Only if the function can panic.)
///
/// # Examples
///
/// ```rust
/// use logicaffeine_{crate}::{function};
///
/// let result = function(arg1, arg2)?;
/// assert_eq!(result, expected);
/// # Ok::<(), Error>(())
/// ```
///
/// # See Also
///
/// - [`related_function`]: {when to use instead}
pub fn my_function(arg1: Type1, arg2: Type2) -> Result<Output, Error> {
    // ...
}
```

### 5.4 Trait Documentation Template

```rust
/// {One-line description of what implementing this trait means.}
///
/// # Implementing
///
/// Types that implement this trait must {requirements/invariants}.
///
/// # Examples
///
/// ```rust
/// use logicaffeine_{crate}::{Trait};
///
/// struct MyType;
///
/// impl Trait for MyType {
///     fn method(&self) -> Output {
///         // ...
///     }
/// }
/// ```
///
/// # Provided Methods
///
/// {List any methods with default implementations.}
pub trait MyTrait {
    /// {Method description.}
    fn required_method(&self) -> Output;

    /// {Description of default behavior.}
    ///
    /// Override this method to {reason}.
    fn provided_method(&self) -> Output {
        // default implementation
    }
}
```

### 5.5 Type Alias Documentation

```rust
/// {What this type alias represents.}
///
/// # Example
///
/// ```rust
/// use logicaffeine_{crate}::{Alias};
///
/// let value: Alias = construct();
/// ```
pub type MyAlias = UnderlyingType<Params>;
```

---

## 6. Code Examples Strategy

### 6.1 Principles

1. **Every public item needs at least one runnable example**
2. **Examples should be minimal but complete**
3. **Examples should compile and pass as tests**
4. **Show common use cases, not exhaustive API coverage**

### 6.2 Doc-Test Configuration

```rust
// Standard runnable example
/// ```rust
/// let x = 1 + 1;
/// assert_eq!(x, 2);
/// ```

// Example that compiles but doesn't run (e.g., I/O, async)
/// ```rust,no_run
/// # fn main() -> std::io::Result<()> {
/// let file = std::fs::read("example.txt")?;
/// # Ok(())
/// # }
/// ```

// Example that doesn't even compile (showing incorrect usage)
/// ```rust,compile_fail
/// let x: u32 = "not a number"; // This fails to compile
/// ```

// Example with expected output
/// ```rust
/// let x = vec![1, 2, 3];
/// println!("{:?}", x);
/// // Output: [1, 2, 3]
/// ```

// Ignore: for examples that require external state/resources
/// ```rust,ignore
/// // Requires Z3 to be installed
/// let verifier = Verifier::new(license_key)?;
/// ```
```

### 6.3 Feature-Gated Example Patterns

```rust
/// Creates a verifier session.
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "verification")]
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use logicaffeine_verify::{Verifier, VerifyExpr};
///
/// let verifier = Verifier::new()?;
/// let result = verifier.check(expr)?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "verification")]
pub fn create_session() -> Result<Session, Error> {
    // ...
}
```

### 6.4 Cross-Crate Example Pattern

```rust
/// # Examples
///
/// Using with the language crate:
///
/// ```rust
/// use logicaffeine_language::compile;
/// use logicaffeine_proof::BackwardChainer;
///
/// let expr = compile("All humans are mortal")?;
/// let proof = BackwardChainer::new()
///     .with_premise(expr)
///     .prove(goal)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
```

### 6.5 Example Priority by Item Type

| Item Type | Examples Required | Example Complexity |
|-----------|-------------------|-------------------|
| Crate root | 1-3 | Medium (full workflow) |
| Public struct | 1 minimum | Minimal (construction) |
| Public enum | 1 (matching) | Minimal |
| Public function | 1-2 | Minimal to medium |
| Public trait | 1 impl + 1 usage | Medium |
| Builder pattern | 1 fluent chain | Medium |
| Error types | 1 (handling) | Minimal |

---

## 7. Cross-Linking Strategy

### 7.1 Intra-Doc Link Patterns

```rust
// Link to item in same module
/// See [`OtherType`] for related functionality.

// Link to item in another module (same crate)
/// The parser returns [`crate::ast::Expr`] nodes.

// Link to item in another crate
/// Uses [`logicaffeine_base::Arena`] for allocation.

// Link to method
/// Internally calls [`Self::validate`] before proceeding.
/// See [`Vec::push`] for similar behavior.

// Link to module
/// For more details, see the [`parser`](crate::parser) module.

// Link to external docs with full URL
/// See the [LOGOS Specification](https://docs.logicaffeine.com/spec) for details.
```

### 7.2 Recommended Cross-Links

#### `logicaffeine-base`
- From `Arena` → `bumpalo::Bump` (external)
- From `Interner` → `Symbol`
- From `Span` → parser usage in `logicaffeine_language`

#### `logicaffeine-lexicon`
- From `VerbClass` → parser verb handling
- From `Time`/`Aspect` → DRS construction

#### `logicaffeine-kernel`
- From `Term` → `Context`
- From `type_checker` → `Term`, `Context`
- From `normalize` → `Term::beta_reduce`
- From `prelude` → all prelude types

#### `logicaffeine-data`
- From CRDTs → `Merge` trait
- From `ORSet` → `DotContext`, `VClock`
- From `types` → kernel types

#### `logicaffeine-system`
- From `distributed` → `logicaffeine_data::crdt`
- From I/O modules → feature flags

#### `logicaffeine-language`
- From `Parser` → `Lexer`, `Token`
- From `compile()` → full pipeline
- From `ast` → `transpile`
- From `proof_convert` → `logicaffeine_proof`

#### `logicaffeine-proof`
- From `BackwardChainer` → `ProofGoal`, `DerivationTree`
- From `InferenceRule` → logic rule explanations
- From hints → `SocraticHint` types

#### `logicaffeine-compile`
- From `compile` → all pipeline stages
- From `ui_bridge` → web integration
- From `verification` → `logicaffeine_verify`

#### `logicaffeine-verify`
- From `Verifier` → Z3 concepts
- From `ir` → language AST conversion
- From `license` → Stripe integration

---

## 8. Conditional Documentation

### 8.1 Feature-Gated Documentation

```rust
// In lib.rs or module files, enable docsrs cfg
#![cfg_attr(docsrs, feature(doc_cfg))]

// Mark feature-gated items
/// Available with the `verification` feature.
#[cfg(feature = "verification")]
#[cfg_attr(docsrs, doc(cfg(feature = "verification")))]
pub mod verification;

// Mark feature-gated re-exports
#[cfg(feature = "networking")]
#[cfg_attr(docsrs, doc(cfg(feature = "networking")))]
pub use network::*;
```

### 8.2 Platform-Specific Documentation

```rust
/// Only available on native targets.
#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(docsrs, doc(cfg(not(target_arch = "wasm32"))))]
pub mod time;

/// Only available on WASM.
#[cfg(target_arch = "wasm32")]
#[cfg_attr(docsrs, doc(cfg(target_arch = "wasm32")))]
pub mod web_time;
```

### 8.3 Target-Specific Examples

```rust
/// # Examples
///
/// Native:
/// ```rust,no_run
/// # #[cfg(not(target_arch = "wasm32"))]
/// # fn example() {
/// use logicaffeine_system::time::now;
/// let timestamp = now();
/// # }
/// ```
///
/// WASM:
/// ```rust,ignore
/// // Use js_sys::Date on WASM targets
/// use logicaffeine_system::web_time::now;
/// ```
```

---

## 9. Building and Hosting

### 9.1 Local Documentation Build

```bash
# Build workspace documentation
cargo doc --workspace --no-deps

# Build with all features
cargo doc --workspace --all-features --no-deps

# Build with private items (as configured)
cargo doc --workspace --no-deps --document-private-items

# Open in browser
cargo doc --workspace --no-deps --open

# Build for specific crate
cargo doc -p logicaffeine-kernel --no-deps --open
```

### 9.2 CI Workflow

Create `.github/workflows/docs.yml`:

```yaml
name: Documentation

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

env:
  CARGO_TERM_COLOR: always
  RUSTDOCFLAGS: "-D warnings --cfg docsrs"

jobs:
  docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        with:
          components: rust-docs

      - name: Build documentation
        run: |
          cargo doc --workspace --all-features --no-deps

      - name: Check for broken intra-doc links
        run: |
          RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

      - name: Deploy to GitHub Pages
        if: github.event_name == 'push' && github.ref == 'refs/heads/main'
        uses: peaceiris/actions-gh-pages@v3
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./target/doc
          destination_dir: rustdoc

  docs-wasm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        with:
          targets: wasm32-unknown-unknown

      - name: Build WASM documentation
        run: |
          cargo doc -p logicaffeine-data --target wasm32-unknown-unknown --no-deps
```

### 9.3 Documentation Lints

Add to workspace `Cargo.toml`:

```toml
[workspace.lints.rust]
missing_docs = "warn"
rustdoc::broken_intra_doc_links = "deny"
rustdoc::private_intra_doc_links = "warn"
rustdoc::invalid_html_tags = "warn"
rustdoc::bare_urls = "warn"

[workspace.lints.clippy]
missing_docs_in_private_items = "allow"  # Only require public docs
```

Each crate inherits:

```toml
[lints]
workspace = true
```

### 9.4 docs.rs Integration

The `[package.metadata.docs.rs]` section in each crate's Cargo.toml controls docs.rs builds:

```toml
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
default-target = "x86_64-unknown-linux-gnu"
targets = ["x86_64-unknown-linux-gnu", "wasm32-unknown-unknown"]
```

---

## 10. Documentation Testing

### 10.1 Running Doc Tests

```bash
# Run all doc tests
cargo test --doc --workspace

# Run doc tests for specific crate
cargo test --doc -p logicaffeine-kernel

# Run doc tests with features
cargo test --doc --workspace --all-features

# Run specific doc test by name
cargo test --doc -p logicaffeine-language "compile"
```

### 10.2 Checking for Broken Links

```bash
# Build with warnings as errors
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Specific link checking
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --workspace --no-deps
```

### 10.3 Coverage Analysis

Use `cargo-doc-coverage` (external tool):

```bash
# Install
cargo install cargo-doc-coverage

# Run coverage check
cargo doc-coverage --workspace
```

Target: 100% public item documentation coverage.

### 10.4 Link Validation Script

Create `scripts/check-docs.sh`:

```bash
#!/bin/bash
set -e

echo "Building documentation..."
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

echo "Running doc tests..."
cargo test --doc --workspace --all-features

echo "Documentation build successful!"
```

---

## 11. Implementation Priority

### Phase 1: Foundation (Week 1)

**Goal**: Fix critical gaps and establish infrastructure.

**Critical Item Documentation:**
- [ ] **`logicaffeine_lexicon`**: Document all 27 `Feature` enum variants (see Section 4.3)
- [ ] **`logicaffeine_lexicon`**: Add field docs to VerbEntry, VerbMetadata, etc.
- [ ] **`logicaffeine_compile`**: Add module-level docs to ui_bridge.rs, codegen.rs, interpreter.rs
- [ ] **`logicaffeine_proof`**: Convert `//` comments to `//!` for rustdoc (minor style fix)

**Infrastructure:**
- [ ] Add workspace-level Cargo.toml metadata
- [ ] Add documentation lints to workspace
- [ ] Create CI workflow for documentation
- [ ] Add `#![cfg_attr(docsrs, feature(doc_cfg))]` to all crates

### Phase 2: Core Crates (Week 2)

**Goal**: Document the foundation layer.

- [ ] `logicaffeine-base`: Complete module docs (4 modules)
  - [ ] `arena` module docs + examples
  - [ ] `intern` module docs + examples
  - [ ] `span` module docs + examples
  - [ ] `error` module docs + examples
- [ ] `logicaffeine-kernel`: Core type theory (12 modules)
  - [ ] `term` module docs + examples
  - [ ] `context` module docs + examples
  - [ ] `type_checker` module docs + examples
  - [ ] Remaining modules (lower priority)
- [ ] `logicaffeine-data`: CRDT documentation (3 modules)
  - [ ] `crdt` module docs + examples
  - [ ] `types` module docs + examples
  - [ ] `indexing` module docs

### Phase 3: Main Crates (Week 3)

**Goal**: Document the language processing pipeline.

- [ ] `logicaffeine-language`: NL→FOL pipeline (~30 modules)
  - [ ] Critical modules: `lexer`, `parser`, `ast`, `token`, `compile`
  - [ ] High-priority: `transpile`, `semantics`, `lambda`, `drs`, `error`
  - [ ] Medium-priority: remaining modules
- [ ] `logicaffeine-proof`: Proof engine (6 modules)
  - [ ] `engine` module docs + examples
  - [ ] `hints` module docs + examples
  - [ ] `unify`, `certifier`, `error`, `oracle`
- [ ] `logicaffeine-compile`: Compilation pipeline (9 modules)
  - [ ] Critical: `compile`, `ui_bridge`
  - [ ] High: `loader`, `analysis`, `codegen`

### Phase 4: Supporting Crates (Week 4)

**Goal**: Complete peripheral crates.

- [ ] `logicaffeine-system`: Platform IO (10+ modules)
  - [ ] Feature-gated documentation for each module
  - [ ] Platform compatibility notes
- [ ] `logicaffeine-lexicon`: Lexicon types (2 modules)
  - [ ] Complete VerbClass/grammatical feature docs
- [ ] `logicaffeine-verify`: Z3 verification (4 modules)
  - [ ] Document Smart Full Mapping strategy
  - [ ] License requirement documentation

### Phase 5: Polish (Week 5)

**Goal**: Cross-linking, diagrams, comprehensive examples.

- [ ] Add cross-crate intra-doc links everywhere
- [ ] Add ASCII diagrams for architecture sections
- [ ] Add comprehensive examples showing full workflows
- [ ] Review all documentation for consistency
- [ ] Final CI integration and docs.rs testing

---

## 12. Style Guide

### 12.1 Tone and Voice

- **Authoritative but accessible**: Assume the reader is a Rust developer but not necessarily a logic expert
- **Active voice**: "The parser produces AST nodes" not "AST nodes are produced by the parser"
- **Imperative mood for function docs**: "Computes the normal form" not "This function computes"
- **Present tense**: "Returns an error if" not "Will return an error if"

### 12.2 Formatting Standards

- **Line length**: Wrap doc comments at ~80 characters for readability
- **Headers**: Use `#` for sections, `##` for subsections
- **Code blocks**: Always specify language (`` ```rust ``)
- **Links**: Prefer intra-doc links over URLs
- **Lists**: Use `-` for unordered, `1.` for ordered

### 12.3 Terminology Consistency

Use these terms consistently across all documentation:

| Term | Use | Avoid |
|------|-----|-------|
| FOL | First-Order Logic | Predicate logic, predicate calculus |
| AST | Abstract Syntax Tree | Parse tree, syntax tree |
| CRDT | CRDT (no expansion needed) | Conflict-free replicated data type (verbose) |
| NL | Natural language | English (too specific) |
| CIC | Calculus of Inductive Constructions | CoC (ambiguous) |
| backward chaining | lowercase | Backward-Chaining, back-chaining |
| type checking | two words | typechecking, type-checking |
| arena allocation | lowercase | Arena Allocation |

### 12.4 Named Invariants

Always use these exact names and format when referencing:

- **MILNER INVARIANT**: Kernel has no path to lexicon
- **LAMPORT INVARIANT**: Data crate has no IO dependencies
- **LISKOV INVARIANT**: Proof crate has no language dependency
- **CERF/DRASNER AMENDMENT**: Feature-gated heavy dependencies

### 12.5 Code Example Style

```rust
// Good: Minimal, focused, compiles
/// ```rust
/// use logicaffeine_kernel::Term;
/// let t = Term::var("x");
/// ```

// Bad: Too verbose, unnecessary setup
/// ```rust
/// // First we need to create a term
/// // Terms are the fundamental building block
/// use logicaffeine_kernel::{Term, Context, Universe};
/// fn main() {
///     let ctx = Context::new();
///     let t = Term::var("x");
///     // Now we have a term!
/// }
/// ```
```

### 12.6 Error Documentation Style

```rust
/// # Errors
///
/// Returns [`ParseError::UnexpectedToken`] if the input contains invalid syntax.
/// Returns [`ParseError::UnterminatedString`] if a string literal is not closed.
```

### 12.7 Panic Documentation Style

```rust
/// # Panics
///
/// Panics if `index` is out of bounds.
/// Panics if called from an async context without a runtime.
```

---

## 13. Documentation Quadrant Model (Diátaxis Framework)

Good documentation serves four distinct purposes. Based on the [Diátaxis framework](https://diataxis.fr/), each requires a different approach:

| Quadrant | Oriented To | Must | Form | logicaffeine Example |
|----------|-------------|------|------|---------------------|
| **Tutorials** | Learning | Allow newcomer to get started | Lesson | "Your First Proof in LOGOS" |
| **How-to Guides** | Goals | Show how to solve specific problem | Steps | "How to Add a Custom Predicate" |
| **Reference** | Information | Describe the machinery | Dry description | API docs, type signatures |
| **Explanation** | Understanding | Explain concepts | Discursive | "Why CRDTs for Collaboration?" |

### 13.1 Current Coverage Assessment

| Quadrant | Status | Examples Present |
|----------|--------|------------------|
| **Tutorials** | Missing | None |
| **How-to Guides** | Missing | None |
| **Reference** | Partial | API docs in progress |
| **Explanation** | Good | README architecture sections, invariants |

### 13.2 Tutorial Roadmap (Learning-oriented)

Tutorials should guide a newcomer through their first successful experience.

```
[ ] "Getting Started with LOGOS" (language crate)
    - Install, configure, "Hello World" proof
    - 15-minute success guarantee

[ ] "Your First Proof" (proof crate)
    - Write premises, set goal, run backward chaining
    - Understand Socratic hints

[ ] "Building a Custom CRDT" (data crate)
    - Implement Merge trait
    - Test convergence properties
```

### 13.3 How-to Guide Roadmap (Goal-oriented)

How-to guides show how to solve specific problems.

```
[ ] "How to compile English to FOL"
    - Input: English sentence
    - Output: Unicode/LaTeX FOL

[ ] "How to verify a theorem with Z3"
    - Setup verification feature
    - License requirements
    - Interpret counterexamples

[ ] "How to sync state between clients"
    - CRDT selection guide
    - Distributed feature flag
    - Conflict resolution

[ ] "How to add a new verb to the lexicon"
    - JSON format
    - Vendler classification
    - Feature flags
```

### 13.4 Reference Documentation (Information-oriented)

Reference docs describe the machinery. This is what rustdoc generates.

```
[x] API documentation (rustdoc) - IN PROGRESS
[x] Type signatures and enum variants
[ ] Error catalogs with solutions (ErrorKind → fix guidance)
[ ] Configuration option reference
```

### 13.5 Explanation Documentation (Understanding-oriented)

Explanations help readers understand concepts and design decisions.

```
[x] Architecture invariants (LAMPORT, MILNER, LISKOV, TARSKI) - DONE
[x] README architecture sections - DONE
[ ] "Why Vendler Classes Matter" - linguistic theory background
[ ] "Understanding Backward Chaining" - proof search algorithm
[ ] "The CRDT Lattice Hierarchy" - mathematical foundations
[ ] "Curry-Howard in Practice" - proofs-as-programs
```

---

## 14. Audience-Aware Writing Guidelines

### 14.1 Primary Audience Profile

Write for Rust developers who:
- Know Rust well (ownership, traits, generics, lifetimes)
- May NOT know formal logic (FOL, predicates, quantifiers)
- May NOT know linguistics (Vendler classes, DRS, theta roles)
- Want to USE the library, not understand every detail first

### 14.2 Writing Principles

| Principle | Do | Don't |
|-----------|-----|-------|
| **Assume Rust knowledge** | Trust the reader knows `impl Trait` | Explain ownership basics |
| **Don't assume logic knowledge** | "Universal quantifier (∀, 'for all')" | "The ∀-introduction rule" |
| **Don't assume linguistics knowledge** | "Vendler class (verb timing patterns)" | "Aktionsart categories" |
| **Link to prerequisites** | "See [Explanation: Lambda]" | Leave jargon unexplained |
| **Start concrete, then abstract** | Example first, theory after | Abstract definition first |

### 14.3 Modular Documentation Pattern

When a concept requires background knowledge, link to explanation docs:

```rust
/// Computes the beta-normal form of a term.
///
/// Beta-reduction substitutes arguments into function bodies. For detailed
/// explanation of the lambda calculus, see the
/// [Lambda Calculus Explanation](crate::explanation::lambda) section.
///
/// # Example
///
/// ```rust
/// use logicaffeine_kernel::normalize;
///
/// let term = parse("(λx.x) y").unwrap();
/// assert_eq!(normalize(&term).to_string(), "y");
/// ```
pub fn normalize(term: &Term) -> Term { ... }
```

### 14.4 Glossary-First Approach

Every domain term should have a glossary entry. Use this pattern:

```rust
/// A [Vendler class] categorizing the temporal structure of verbs.
///
/// [Vendler class]: crate::glossary::vendler_class
pub enum VerbClass { ... }
```

### 14.5 Newcomer Validation Process

Before marking documentation complete:

1. **Find a reviewer**: Rust dev unfamiliar with logic/linguistics
2. **Give them a task**: "Use this library to prove X"
3. **Observe silently**: Note where they get stuck
4. **Update documentation**: Address confusion points
5. **Repeat**: Until a newcomer can succeed independently

This validation catches assumptions that experts make unconsciously.

### 14.6 Error Message as Documentation

Every error variant should explain:
1. What went wrong
2. Why it's a problem
3. How to fix it

```rust
/// Type mismatch during proof construction.
///
/// # What went wrong
/// The term's actual type doesn't match the expected type.
///
/// # Why it's a problem
/// Type-incorrect proofs can't be certified by the kernel.
///
/// # How to fix
/// Check that your premise types match your goal. Use `infer_type()`
/// to see what type the kernel inferred.
TypeMismatch {
    expected: Type,
    actual: Type,
    context: String,
}
```

---

## Appendix A: Quick Reference Card

### Minimum Documentation Checklist

```text
[ ] Crate-level //! docs with Quick Start
[ ] Module-level //! docs for each module
[ ] All public structs documented with examples
[ ] All public enums documented with variant docs
[ ] All public functions documented with Arguments/Returns/Errors/Examples
[ ] All public traits documented with Implementing section
[ ] Feature flags documented with #[doc(cfg(...))]
[ ] Cross-crate links working
[ ] Doc tests passing
```

### Command Cheat Sheet

```bash
# Build all docs
cargo doc --workspace --all-features --no-deps --open

# Run doc tests
cargo test --doc --workspace

# Check for warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Single crate
cargo doc -p logicaffeine-kernel --open
```

---

## Appendix B: Example Crate Header

Complete example of a well-documented crate header:

```rust
//! # Logicaffeine Kernel
//!
//! Pure Calculus of Constructions type theory implementation.
//!
//! This crate provides the foundational type system for LOGOS, implementing
//! a variant of the Calculus of Inductive Constructions (CIC). All terms,
//! types, and proofs share the same syntactic category.
//!
//! ## Quick Start
//!
//! ```rust
//! use logicaffeine_kernel::{Term, Context, infer_type};
//!
//! // Create a simple term
//! let identity = Term::lambda("x", Term::var("x"));
//!
//! // Type check it
//! let ctx = Context::new();
//! let ty = infer_type(&ctx, &identity)?;
//! # Ok::<(), logicaffeine_kernel::KernelError>(())
//! ```
//!
//! ## Feature Flags
//!
//! This crate has no feature flags. All functionality is always available.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────┐     ┌─────────────┐     ┌──────────┐
//! │   Term   │────▶│   Context   │────▶│  Result  │
//! └──────────┘     └─────────────┘     └──────────┘
//!       │                │
//!       ▼                ▼
//! ┌──────────┐     ┌─────────────┐
//! │ Reduction│     │ Type Check  │
//! └──────────┘     └─────────────┘
//! ```
//!
//! ## MILNER INVARIANT
//!
//! This crate has **NO** path to the lexicon. Adding words to the English
//! vocabulary never triggers a recompile of the type checker. The kernel
//! knows nothing about natural language—it only knows about pure type theory.
//!
//! ## See Also
//!
//! - [`logicaffeine_base`]: Foundation types used by this crate
//! - [`logicaffeine_proof`]: Uses this crate for proof certification
//! - [CIC Paper](https://hal.inria.fr/inria-00076024): Original theory

#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(missing_docs)]

mod context;
mod error;
// ... etc
```

---

## Appendix C: Executable Agent Task Specifications

This appendix provides copy-paste ready task specifications for AI coding assistants to execute documentation improvements.

### C.1 Agent Task: Document Feature Enum

```
TASK: Document logicaffeine_lexicon Feature enum
FILE: crates/logicaffeine_lexicon/src/types.rs
PATTERN: For each variant, add a doc comment with:
  1. One-line summary
  2. Linguistic explanation (2-3 sentences)
  3. Example words in English

TEMPLATE:
/// {One-line summary ending with period.}
///
/// {2-3 sentence explanation of what this feature represents linguistically.
/// Include when this feature applies and what it indicates about the word.}
///
/// Examples: "{word1}", "{word2}", "{word3}"
VariantName,

EXECUTION:
1. Read the entire Feature enum
2. For each undocumented variant, research its linguistic meaning
3. Write the doc comment following the template
4. Preserve existing variant ordering
5. Run `cargo doc -p logicaffeine-lexicon --open` to verify
```

### C.2 Agent Task: Document Compile Module

```
TASK: Add module-level documentation to logicaffeine_compile
FILES:
  - crates/logicaffeine_compile/src/ui_bridge.rs
  - crates/logicaffeine_compile/src/codegen.rs
  - crates/logicaffeine_compile/src/interpreter.rs
  - crates/logicaffeine_compile/src/extraction/mod.rs
  - crates/logicaffeine_compile/src/analysis/escape.rs
  - crates/logicaffeine_compile/src/analysis/ownership.rs

TEMPLATE (at top of file, before any `use` statements):
//! # {Module Name}
//!
//! {One paragraph describing the module's purpose and role in the compilation pipeline.}
//!
//! ## Overview
//!
//! {Key types and their relationships. What does this module do?}
//!
//! ## Key Types
//!
//! - [`TypeName`]: {brief description}
//!
//! ## See Also
//!
//! - [`related_module`]: {relationship}

EXECUTION:
1. Read each file entirely to understand its purpose
2. Identify the key public types and functions
3. Write module-level docs following the template
4. Ensure cross-references use proper intra-doc link syntax
5. Run `cargo doc -p logicaffeine-compile --open` to verify links
```

### C.3 Agent Task: Document Struct Fields

```
TASK: Add field documentation to lexicon structs
FILE: crates/logicaffeine_lexicon/src/types.rs
STRUCTS: VerbEntry, VerbMetadata, NounMetadata, AdjectiveMetadata, CanonicalMapping, MorphologicalRule

TEMPLATE:
pub struct StructName {
    /// {Description of what this field represents.}
    ///
    /// {Optional: constraints, valid values, relationships to other fields.}
    pub field_name: FieldType,
}

EXECUTION:
1. For each struct, identify all public fields
2. Add doc comment to each field explaining:
   - What the field represents
   - Any constraints or valid values
   - Relationship to other fields if relevant
3. Run `cargo doc -p logicaffeine-lexicon --open` to verify
```

### C.4 Agent Task: Convert Comments to Doc Comments

```
TASK: Convert logicaffeine_proof lib.rs comments to doc comments
FILE: crates/logicaffeine_proof/src/lib.rs

PATTERN:
  BEFORE: // Some explanation text
  AFTER:  //! Some explanation text

RULES:
1. Only convert comments at the top of the file (before `use` statements)
2. Convert `// ===` header bars to `//! ` (keep the visual structure)
3. Convert all `// ` explanatory text to `//! `
4. Do NOT convert comments inside functions or after code
5. Preserve all existing text exactly, only change comment prefix

EXECUTION:
1. Read the file and identify the crate-level documentation section
2. Convert `//` to `//!` for all lines in that section
3. Stop converting when you reach `use` statements or `mod` declarations
4. Run `cargo doc -p logicaffeine-proof --open` to verify docs appear
```

### C.5 Agent Task: Add License Module Documentation

```
TASK: Document license validation in logicaffeine_verify
FILE: crates/logicaffeine_verify/src/license.rs

ITEMS TO DOCUMENT:
1. LicenseValidator struct
2. LicensePlan enum (all variants)
3. validate() method
4. Any cache-related types or constants

TEMPLATE for LicensePlan:
/// Subscription tier determining available verification features.
///
/// License plans control access to Z3-based verification functionality.
/// Plans are validated against the Stripe subscription API.
pub enum LicensePlan {
    /// Free tier with limited verification capabilities.
    Free,
    /// Professional tier with full verification access.
    Pro,
    // ... etc
}

TEMPLATE for LicenseValidator:
/// Validates license keys against the Stripe subscription API.
///
/// Maintains a local cache with 24-hour TTL to reduce API calls.
/// Cache is stored at `{config_dir}/logicaffeine/license_cache.json`.
///
/// # Example
///
/// ```rust,ignore
/// let validator = LicenseValidator::new()?;
/// match validator.validate("sub_xxx")? {
///     LicensePlan::Pro => { /* full access */ }
///     LicensePlan::Free => { /* limited access */ }
/// }
/// ```
pub struct LicenseValidator { ... }

EXECUTION:
1. Read the entire license.rs file
2. Document each public item following the templates
3. Include cache TTL (24 hours) in documentation
4. Include cache file location in documentation
5. Run `cargo doc -p logicaffeine-verify --open` to verify
```

### C.6 Verification Checklist

After completing any documentation task:

```bash
# 1. Build docs for the modified crate
cargo doc -p logicaffeine-{crate} --no-deps

# 2. Check for warnings (should be zero)
RUSTDOCFLAGS="-D warnings" cargo doc -p logicaffeine-{crate} --no-deps

# 3. Run doc tests
cargo test --doc -p logicaffeine-{crate}

# 4. Open and visually inspect
cargo doc -p logicaffeine-{crate} --no-deps --open
```

All four commands must pass before marking a documentation task complete.
