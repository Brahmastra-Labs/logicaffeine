# Logicaffeine: Tiered Crates Architecture Plan
 REMEMBER STRANGLER FIG
 
## 1. Executive Vision

To elevate `logicaffeine` from a monolithic research prototype into a **production-grade language ecosystem**. We adopt a **"Tiered Crates"** architecture that respects natural module coupling boundaries, enabling users to pull in only what they need while keeping tightly-coupled modules together.

## 1.5 Architectural Decisions (Council Recommendations)

This section documents key architectural decisions validated by audit.

### The Kernel Isolation Principle
**Status: APPROVED (Turing/Milner)**

The `logicaffeine_kernel` crate (pure Calculus of Constructions) has NO path to lexicon.
- Kernel compiles with `deps: base ONLY`
- Adding "skibidi" to the dictionary does not recompile the type checker
- Third-party tools can verify proofs without loading the English parser

### The Proof-Language Coupling
**Status: ACKNOWLEDGED (Liskov's Observation)**

`logicaffeine_proof` depends on `logicaffeine_language` for two specific reasons:

1. **AST Conversion** (`convert.rs`):
   - Bridges parser's arena-allocated `LogicExpr` → proof engine's owned `ProofExpr`
   - Imports: `ast::logic`, `lexicon::get_canonical_noun`, `token::TokenType`

2. **Hint Text Generation** (`hints.rs`):
   - Generates pedagogical English strings
   - BUT hints ARE structured (not just strings):
     ```rust
     pub struct SocraticHint {
         pub text: String,                          // English question
         pub suggested_tactic: Option<SuggestedTactic>,  // Enum with 11 variants
         pub priority: u8,                          // 0-10 relevance score
     }
     ```

**Decoupling Strategy**: See **Section 1.6 (The Liskov Trait Amendment)** for the v1.0 implementation plan. This is NOT deferred - it is part of Phase 3.3.

### The Dual Lexicon System
**Status: EXISTS (Knuth's Concern Addressed)**

Two parallel implementations already exist:

| Path | File | Mechanism | Use Case |
|------|------|-----------|----------|
| Compiled | `src/lexicon.rs` | Generated static match expressions | Production, WASM |
| Runtime | `src/runtime_lexicon.rs` | `serde_json::from_str(include_str!(...))` | Development, testing |

The runtime lexicon provides:
- `LexiconIndex::new()` - parses JSON at startup
- `.verbs_with_feature()`, `.nouns_with_sort()` - rich filtering
- `.random_transitive_verb()` - test generation

**Action**: Expose via `dynamic-lexicon` feature flag for faster debug builds.

### The Z3 Exclusion Pattern
**Status: ALREADY IMPLEMENTED (Drasner/Hopper Alert)**

Current `Cargo.toml` line 24:
```toml
[workspace]
exclude = ["logos_core", "logos_verification"]
```

This ensures:
- `cargo build --workspace` works without Z3
- `cargo test --workspace` doesn't require Z3 C++ libraries
- Verification is opt-in: `cargo build -p logos --features verification`

### The WASM Data Separation
**Status: CORRECT (Lamport/Victor Praise)**

`logicaffeine_data` uses vector clocks (`crdt/causal/vclock.rs`) with:
- `HashMap<ReplicaId, u64>` - no system time
- `increment()`, `dominates()`, `concurrent()` - pure logical operations
- Time as external input

Browser clients get CRDTs (~120KB) without libp2p.

### Vector Clock Purity Audit
**Status: VERIFIED (Lamport's Concern)**

Audit of `logos_core/src/crdt/causal/` confirms:
- `vclock.rs`: Uses only `HashMap<ReplicaId, u64>` - pure logical counters
- `dot.rs`: Uses `(ReplicaId, u64)` pairs - no time dependency
- `context.rs`: Pure logical operations for dot contiguity

**System time is isolated to:**
- `replica.rs`: ID generation only (not ordering)
- `lww.rs`: LWWRegister (explicit time semantics by design)

**LWW Exception**: `lww.rs` currently uses `std::time::SystemTime` directly.
This must be refactored to use time injection (see "The LWW Time Injection Pattern" below)
before the data crate can compile for WASM.

**build.rs Audit:** Line 237 correctly declares `cargo:rerun-if-changed=assets/lexicon.json`

### The LWW Time Injection Pattern
**Status: REQUIRED FIX (Lamport Mandate)**

The `LWWRegister` in `logicaffeine_data` must NOT import `std::time::SystemTime`.
The data crate stays pure; the system crate provides timestamps.

**Current Code (Violates Purity)**:
```rust
// logos_core/src/crdt/lww.rs - WRONG
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}
```

**Required Refactor (Pure)**:
```rust
// crates/logicaffeine_data/src/crdt/lww.rs - CORRECT
pub struct LWWRegister<T> {
    value: T,
    timestamp: u64,  // Pure integer, no SystemTime
}

impl<T> LWWRegister<T> {
    /// Create with explicit timestamp (pure, no IO)
    pub fn new(value: T, timestamp: u64) -> Self {
        Self { value, timestamp }
    }

    /// Set with explicit timestamp (pure, no IO)
    pub fn set(&mut self, value: T, timestamp: u64) {
        if timestamp >= self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
        }
    }
}
```

**Caller Responsibility** (`logicaffeine_system`):
```rust
// crates/logicaffeine_system/src/time.rs
#[cfg(not(target_arch = "wasm32"))]
pub fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as u64
}

#[cfg(target_arch = "wasm32")]
pub fn now_micros() -> u64 {
    (js_sys::Date::now() * 1000.0) as u64  // ms to μs
}
```

**Architectural Alignment**:
- `logicaffeine_data`: "I store value X at time T." (Pure)
- `logicaffeine_system`: "The time is now T." (Impure, platform-specific)

**Implementation Phase**: This fix is part of **Phase 2.1: Data Extraction**.

### The Z3 Exclusion (Empathy Engineering)
**Status: VERIFIED (Hanselman's Approval)**

By excluding `logos_verification` from the workspace, new contributors can:
- Clone the repo
- Run `cargo test`
- Get a working build

...without spending 4 hours installing LLVM bindings or Z3 C++ dependencies.

---

## 1.6 The Liskov Trait Amendment
**Status: APPROVED FOR v1.0 (Council Mandate)**

The `HintRenderer` trait loosens the coupling between `logicaffeine_proof` and `logicaffeine_language` while preserving the pedagogical connection. The proof crate becomes self-contained with `DefaultHintRenderer`, while the language crate provides `EnglishHintRenderer` for richer pedagogical hints. BOTH are implemented in v1.0.

### Trait Definition (in `logicaffeine_proof/src/hints.rs`)

```rust
/// Trait for rendering structured hints into human-readable text.
/// This enables language-agnostic proof guidance.
pub trait HintRenderer {
    /// Render a suggested tactic name (e.g., "Modus Ponens" -> "Try Modus Ponens!")
    fn render_tactic(&self, tactic: SuggestedTactic) -> String;

    /// Render a full Socratic hint with context
    fn render_hint(&self, hint: &SocraticHint) -> String;

    /// Render goal structure description (e.g., "Your goal is an implication P → Q")
    fn render_goal_structure(&self, goal: &ProofExpr) -> Option<String>;
}

/// Default implementation provides basic structured English text.
/// No lexicon required - proof crate is fully self-contained.
pub struct DefaultHintRenderer;

impl HintRenderer for DefaultHintRenderer {
    fn render_tactic(&self, tactic: SuggestedTactic) -> String {
        format!("Try {}.", tactic.name())
    }

    fn render_hint(&self, hint: &SocraticHint) -> String {
        hint.text.clone()
    }

    fn render_goal_structure(&self, _goal: &ProofExpr) -> Option<String> {
        None // Uses built-in analyze_goal_structure()
    }
}
```

### Enhanced Renderer (in `logicaffeine_language`) — ALSO REQUIRED

```rust
// crates/logicaffeine_language/src/hints/renderer.rs
use logicaffeine_proof::{HintRenderer, SocraticHint, SuggestedTactic, ProofExpr};
use crate::lexicon::LexiconIndex;

/// Richer English hint renderer that uses the lexicon for natural phrasing.
pub struct EnglishHintRenderer {
    lexicon: &'static LexiconIndex,
}

impl HintRenderer for EnglishHintRenderer {
    fn render_tactic(&self, tactic: SuggestedTactic) -> String {
        match tactic {
            SuggestedTactic::ModusPonens =>
                "If you have P and P→Q, you can derive Q using Modus Ponens.".into(),
            SuggestedTactic::UniversalElim =>
                "You have a universal statement. What specific value should you substitute?".into(),
            // ... richer English descriptions using lexicon vocabulary
            _ => format!("Consider using {}.", tactic.name())
        }
    }
    // ...
}
```

### Dependency Impact

| Before | After |
|--------|-------|
| `proof` → hard dependency → `language` | `proof` self-contained with `DefaultHintRenderer` |
| Cannot use proof without English parser | Can use proof standalone for type theory |
| Coupling blocks independent versioning | Crates can version independently |

### TDD Test Specification

```rust
// tests/phase_liskov_trait.rs
#[test]
fn proof_crate_compiles_without_language() {
    // RED: Proof crate currently imports language for hints.rs
    // GREEN: DefaultHintRenderer provides all hint text internally
    // Verify: `cargo build -p logicaffeine_proof` succeeds
    //         `cargo tree -p logicaffeine_proof | grep language` returns EMPTY
}

#[test]
fn default_renderer_provides_all_tactics() {
    // Verify all 11 SuggestedTactic variants render to non-empty strings
    let renderer = DefaultHintRenderer;
    for tactic in [ModusPonens, UniversalElim, ExistentialIntro, AndIntro,
                   AndElim, OrIntro, OrElim, Induction, Reflexivity, Rewrite, Assumption] {
        assert!(!renderer.render_tactic(tactic).is_empty());
    }
}

#[test]
fn english_renderer_uses_lexicon() {
    // Only runs when language crate is available
    // Verifies EnglishHintRenderer produces richer output than default
}
```

### Implementation Phase

This amendment is implemented during **Phase 3.3: Proof Extraction**. ALL steps are REQUIRED for v1.0:

1. Define `HintRenderer` trait in `hints.rs`
2. Implement `DefaultHintRenderer` with all hint text inline (proof is self-contained)
3. Remove imports from `language` crate in proof module
4. Verify: `cargo tree -p logicaffeine_proof | grep language` returns empty
5. Create `EnglishHintRenderer` in language crate (richer UX for CLI/Web)
6. Wire up: CLI and Web use `EnglishHintRenderer`, standalone proof users get `DefaultHintRenderer`

**Note**: "Optional dependency" means the proof crate CAN be used without language at runtime. It does NOT mean we skip implementing the EnglishHintRenderer.

---

## 1.7 The Cerf/Drasner Feature Flags
**Status: APPROVED FOR v1.0 (Council Mandate) — TO BE IMPLEMENTED**

> **Note**: Current `logos_core/Cargo.toml` has no feature flags. All native
> dependencies (libp2p, tokio, rayon) are always compiled. This section
> describes the TARGET architecture, not the current state.

The `logicaffeine_system` crate uses feature flags to enable users to opt out of heavy dependencies they don't need. A simple script runner shouldn't require the GossipSub networking stack.

### Feature Flag Structure

```toml
# crates/logicaffeine_system/Cargo.toml

[package]
name = "logicaffeine-system"
version = "0.1.0"
edition = "2021"

[features]
default = []  # LEAN by default - no network, no persistence, no parallelism

# Individual capabilities (opt-in)
networking = ["libp2p", "futures"]
persistence = ["memmap2", "sha2"]
concurrency = ["rayon"]

# Convenience bundles
full = ["networking", "persistence", "concurrency"]
distributed = ["networking", "persistence"]  # For Distributed<T>

[dependencies]
# Always included (lightweight core)
logicaffeine-base = { path = "../logicaffeine_base" }
logicaffeine-data = { path = "../logicaffeine_data" }
async-trait = "0.1"
once_cell = "1.19"
async-lock = "3.4"

# Feature-gated heavy dependencies
libp2p = { version = "0.54", optional = true, features = [
    "tcp", "quic", "noise", "yamux", "mdns",
    "request-response", "gossipsub", "macros", "tokio"
] }
memmap2 = { version = "0.9", optional = true }
sha2 = { version = "0.10", optional = true }
rayon = { version = "1.10", optional = true }
futures = { version = "0.3", optional = true }

# Native-only (always available on native, not WASM)
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
```

### Module Conditional Compilation

```rust
// crates/logicaffeine_system/src/lib.rs

// Always available (core IO)
pub mod io;
pub mod time;
pub mod env;
pub mod random;

// Feature-gated modules
#[cfg(feature = "networking")]
pub mod network;

#[cfg(feature = "persistence")]
pub mod storage;
#[cfg(feature = "persistence")]
pub mod fs;
#[cfg(feature = "persistence")]
pub mod file;

#[cfg(feature = "concurrency")]
pub mod concurrency;
#[cfg(feature = "concurrency")]
pub mod memory;

// Distributed<T> requires both networking and persistence
#[cfg(all(feature = "networking", feature = "persistence"))]
pub mod distributed;
```

### Updated User Scenarios

| Use Case | System Features | Binary Size Delta | Dependencies |
|----------|-----------------|-------------------|--------------|
| Simple script runner | (none) | +~50KB | async-trait, once_cell |
| Local file processing | `persistence` | +~120KB | +memmap2, sha2 |
| Parallel computation | `concurrency` | +~80KB | +rayon |
| P2P collaborative app | `distributed` | +~2MB | +libp2p, memmap2 |
| Full server | `full` | +~2.2MB | Everything |

### TDD Test Specification

```rust
// tests/phase_cerf_drasner_flags.rs

#[test]
fn system_compiles_with_no_features() {
    // RED: System crate currently requires libp2p unconditionally
    // GREEN: Default features = [], io.rs compiles standalone
    // Verify: `cargo build -p logicaffeine_system` succeeds
}

#[test]
#[cfg(feature = "networking")]
fn networking_feature_enables_gossip() {
    use logicaffeine_system::network::gossip;
    // Only runs when networking feature is enabled
}

#[test]
#[cfg(feature = "persistence")]
fn persistence_feature_enables_storage() {
    use logicaffeine_system::storage;
    // Only runs when persistence feature is enabled
}

#[test]
#[cfg(all(feature = "networking", feature = "persistence"))]
fn distributed_requires_both_features() {
    use logicaffeine_system::distributed::Distributed;
    // Only compiles when both features are enabled
}
```

### Implementation Phase

This amendment is implemented during **Phase 3.1: System Extraction**. The extraction steps are:

1. Create feature flags in `Cargo.toml`
2. Add `#[cfg(feature = "...")]` guards to modules
3. Move heavy dependencies behind optional flags
4. Verify: `cargo build -p logicaffeine_system` with no features
5. Verify: Each feature combination compiles correctly

---

## 1.8 The Knuth Documentation Standard
**Status: APPROVED FOR v1.0 (Council Mandate)**

The root `Cargo.toml` must configure docs.rs to build all workspace crates together (excluding verify) so documentation shows the interplay between crates. Users should see unified, cross-linked documentation.

### Root Cargo.toml Configuration

```toml
# Cargo.toml (workspace root)

[workspace]
resolver = "2"
members = [
    "crates/logicaffeine_base",
    "crates/logicaffeine_lexicon",
    # ... all crates
]
exclude = ["crates/logicaffeine_verify"]

# KNUTH DOCUMENTATION STANDARD
[workspace.metadata.docs.rs]
# Build all workspace crates together for unified docs
all-features = false  # Don't include verify feature (requires Z3)
features = ["cli"]    # Include CLI documentation
cargo-args = ["--workspace"]
rustdoc-args = ["--document-private-items"]

[workspace.dependencies]
# Shared versions ensure doc cross-references work
bumpalo = "3.19"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

### Per-Crate docs.rs Configuration

Each crate should document with its full feature set:

```toml
# crates/logicaffeine_system/Cargo.toml

[package.metadata.docs.rs]
# Document with all features for comprehensive API coverage
features = ["full"]
```

```toml
# crates/logicaffeine_language/Cargo.toml

[package.metadata.docs.rs]
# Include dynamic-lexicon docs for developers
features = ["dynamic-lexicon"]
```

### Documentation Cross-References

The generated docs should include:
- **Re-exports**: Show crate relationships via `pub use`
- **Feature flags**: Document which features enable which APIs
- **Examples**: Inter-crate usage patterns

### CI Verification

```yaml
# .github/workflows/docs.yml
name: Documentation
on: [push, pull_request]

jobs:
  docs:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build Documentation
        run: |
          cargo doc --workspace --no-deps --exclude logicaffeine_verify
          # Verify no broken intra-doc links
      - name: Check for broken links
        run: |
          cargo install cargo-deadlinks || true
          cargo deadlinks --check-http || echo "Some external links may be unavailable"
```

### Implementation Phase

This amendment is implemented during **Phase 1: Workspace Infrastructure**:

1. Add `[workspace.metadata.docs.rs]` to root `Cargo.toml`
2. Add `[package.metadata.docs.rs]` to each crate's `Cargo.toml`
3. Verify: `cargo doc --workspace --no-deps` succeeds
4. Add docs CI workflow

---

## 1.9 The Web Diet Amendment
**Status: APPROVED FOR v1.0 (Papert/Primeagen Mandate)**

The `logicaffeine_compile` crate uses feature flags to exclude the heavy Rust codegen
backend when building for WASM. The web playground only needs the interpreter for
immediate feedback; it doesn't need to generate standalone Rust binaries.

### Feature Flag Structure

```toml
# crates/logicaffeine_compile/Cargo.toml

[package]
name = "logicaffeine-compile"
version = "0.1.0"
edition = "2021"

[features]
default = ["codegen"]  # Full compilation for CLI

# Individual capabilities
codegen = []           # Rust code generation (compile.rs, diagnostic.rs, sourcemap.rs)
interpreter-only = []  # Tree-walking interpreter only (no codegen)

[dependencies]
# Always included (core analysis + interpreter)
logicaffeine-language = { path = "../logicaffeine_language" }
logicaffeine-data = { path = "../logicaffeine_data" }
logicaffeine-system = { path = "../logicaffeine_system" }

# Feature-gated: codegen modules have platform-specific deps
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
# These are only needed for codegen, not interpreter
```

### Module Conditional Compilation

```rust
// crates/logicaffeine_compile/src/lib.rs

// Always available (analysis + interpreter)
pub mod analysis;
pub mod interpreter;

// Feature-gated (codegen only, native only)
#[cfg(all(feature = "codegen", not(target_arch = "wasm32")))]
pub mod compile;
#[cfg(all(feature = "codegen", not(target_arch = "wasm32")))]
pub mod codegen;
#[cfg(all(feature = "codegen", not(target_arch = "wasm32")))]
pub mod diagnostic;
#[cfg(all(feature = "codegen", not(target_arch = "wasm32")))]
pub mod sourcemap;
#[cfg(all(feature = "codegen", not(target_arch = "wasm32")))]
pub mod extraction;
```

### User Scenarios

| Use Case | Features | Binary Size Delta | Modules Included |
|----------|----------|-------------------|------------------|
| Web playground | `interpreter-only` | ~150KB | analysis, interpreter |
| CLI `largo build` | `codegen` (default) | ~350KB | analysis, interpreter, codegen, compile |
| Full dev environment | `codegen` | ~350KB | Everything |

### Why This Matters (Papert's Law)

The child types in the browser. They expect instant feedback. If we ship the
Rust codegen backend to the browser, we add ~200KB to the WASM bundle for
code they will never execute. This violates the principle of "low floor, high ceiling."

- **Low floor**: Web users get immediate feedback with interpreter
- **High ceiling**: CLI users get full compilation to native binaries

### TDD Test Specification

```rust
// tests/phase_web_diet.rs

#[test]
fn compile_crate_builds_interpreter_only() {
    // Verify: `cargo build -p logicaffeine_compile --features interpreter-only`
    // Must succeed without codegen module errors
}

#[test]
#[cfg(feature = "codegen")]
fn codegen_feature_enables_compile_module() {
    use logicaffeine_compile::compile;
    // Only runs when codegen feature is enabled
}

#[test]
fn web_build_excludes_codegen() {
    // Verify: `cargo build -p logicaffeine_web --target wasm32-unknown-unknown`
    // Must NOT include compile.rs, codegen.rs, diagnostic.rs, sourcemap.rs
}
```

### Implementation Phase

This amendment is implemented during **Phase 3.4: Compile Extraction**:

1. Create feature flags in `Cargo.toml`
2. Add `#[cfg(feature = "codegen")]` guards to codegen modules
3. Add `#[cfg(not(target_arch = "wasm32"))]` guards for native-only code
4. Verify: `cargo build -p logicaffeine_compile --features interpreter-only` succeeds
5. Verify: Web build excludes codegen modules

---

## 2. Architectural Tiers

### Tier 0: The Atoms (`logicaffeine_base`)
Pure structural atoms. **No English. No IO.**
- **Components**: Arena allocation, symbol interning, token types, span tracking, base errors
- **Dependencies**: `bumpalo` only
- **Constraint**: Zero knowledge of English vocabulary. Zero IO.
- **Usage**: EVERYTHING depends on this. The true foundation.

### Tier 0.5: The Dictionary (`logicaffeine_lexicon`)
Generated static arrays from `lexicon.json`. **English knowledge lives here.**
- **Components**: 40+ lookup functions, MWE data, ontology data, axiom data
- **Build-time**: `build.rs` (1,664 lines) processes `lexicon.json`
- **Runtime alternative**: `runtime.rs` (existing `runtime_lexicon.rs`) for dynamic loading
- **Dependencies**: `logicaffeine_base`
- **Build Dependencies**: `serde`, `serde_json` (compile-time only)
- **Feature Flags**:
  - (default): Generated static match expressions (fast, WASM-safe)
  - `dynamic-lexicon`: Load JSON at runtime (faster compiles, dev-friendly)
- **Critical**: The KERNEL does NOT depend on this. Adding "skibidi" to the dictionary does not recompile the type checker.

### Tier 1: Isolated Cores (No cross-dependencies between them)

#### `logicaffeine_kernel` — The Auditor
Pure Calculus of Constructions implementation.
- **Components**: Term representation, type checking, reduction, solver tactics (ring, LIA, omega, congruence closure)
- **Dependencies**: `logicaffeine_base` ONLY (NOT lexicon!)
- **Constraint**: Zero natural language dependencies. Zero lexicon. Publishable standalone.
- **Superpower**: Third-party tools can verify proofs without loading English parser.

#### `logicaffeine_data` — The Shape
Pure data structures. WASM-safe.
- **Components**: Native types (Nat, Int, Real, Text), CRDTs (20+ implementations), Value enum
- **Dependencies**: `logicaffeine_base` ONLY
- **Constraint**: NO IO. NO NETWORK. Pure data structures.
- **Superpower**: Browser clients can use CRDTs without downloading libp2p.
- **Time Note**: Vector clocks (`VClock`, `Dot`, `DotContext`) use pure logical time.
  `LWWRegister` uses wall-clock time BY DESIGN for last-write-wins semantics.

#### `logicaffeine_system` — The Body
Platform IO. The "real world" interface.
- **Components**: VFS abstraction, P2P networking (libp2p), persistence, concurrency
- **Dependencies**: `logicaffeine_base` + `logicaffeine_data`
- **Native deps**: `tokio`, `libp2p`, `memmap2`
- **WASM deps**: `wasm-bindgen`, `web-sys` (OPFS)
- **Constraint**: Heavy. Platform-specific.

### Tier 2: Composite Crates

#### `logicaffeine_proof` — The Reasoner
Backward-chaining proof engine with Socratic hints.
- **Components**: Proof search, unification, certification, hint generation, `HintRenderer` trait
- **Dependencies**: `logicaffeine_kernel` ONLY (Liskov Trait: NO language dependency)
- **Exports**: `HintRenderer` trait, `DefaultHintRenderer`, `SocraticHint`, `SuggestedTactic`
- **Hint Structure**:
  ```rust
  pub struct SocraticHint {
      pub text: String,
      pub suggested_tactic: Option<SuggestedTactic>,  // 11 variants
      pub priority: u8,  // 0-10
  }

  pub enum SuggestedTactic {
      ModusPonens, UniversalElim, ExistentialIntro, AndIntro, AndElim,
      OrIntro, OrElim, Induction, Reflexivity, Rewrite, Assumption,
  }
  ```
- **Why kernel-only**: Pure proof logic. Type theory users can use this without English parser.

#### `logicaffeine_language` — The Linguist
Natural language to first-order logic pipeline.
- **Components**: Lexer (85KB), parser (10 submodules), AST (4 modules), transpiler, semantic axioms, Kripke lowering, lambda calculus (65KB), session management, MWE handling, ontology, pragmatics, `EnglishHintRenderer`
- **Dependencies**: `logicaffeine_base` + `logicaffeine_lexicon` + `logicaffeine_proof`
- **Provides**: `EnglishHintRenderer` (implements `HintRenderer` from proof crate)
- **Usage**: Parse English. Output logic. Rich hints for pedagogy.

### Tier 3: The Builder

#### `logicaffeine_compile` — The Builder
LOGOS imperative language compilation.
- **Components**: Static analysis (7 passes), Rust codegen (113KB), tree-walking interpreter (70KB), program extraction
- **Dependencies**: `logicaffeine_language` + `logicaffeine_data` + `logicaffeine_system`
- **Feature Flags**:
  - `codegen` (default) — Full Rust code generation
  - `interpreter-only` — Lightweight mode for web
- **Why separate**: Compilation pipeline is distinct from FOL parsing.

### Tier 3.5: Optional Extensions

#### `logicaffeine_verify` — The Oracle
Z3-based static verification (license-gated).
- **Components**: Z3 bindings, IR translation, license validation
- **Dependencies**: `logicaffeine_base` + `logicaffeine_kernel` + z3

### Tier 4: Applications

#### `logicaffeine_cli` (`largo`)
Native command-line tool.
- **Dependencies**: All crates + clap, toml, ureq

#### `logicaffeine_web`
Browser-based IDE.
- **Core Dependencies**: `logicaffeine_language` + `logicaffeine_proof` + `logicaffeine_data`
- **Optional**: `logicaffeine_compile` (feature-gated for imperative mode)
- **Feature Flags**:
  - `logic-mode` (default) — Parse + prove only
  - `build-mode` — Add compilation
  - `full` — Everything

## 3. Workspace Structure

```text
logicaffeine/
├── Cargo.toml                      # Virtual Workspace
├── crates/
│   ├── logicaffeine_base/          # TIER 0: Pure Atoms
│   │   ├── src/
│   │   │   ├── lib.rs              # Exports + Interner
│   │   │   ├── arena.rs            # Bump allocation
│   │   │   ├── intern.rs           # Symbol interning (EXTRACTED from lib.rs)
│   │   │   ├── span.rs             # Source location tracking
│   │   │   ├── token.rs            # Token types (EXTRACTED from lexer.rs)
│   │   │   └── error.rs            # Base error types
│   │   └── Cargo.toml              # deps: bumpalo
│   │
│   ├── logicaffeine_lexicon/       # TIER 0.5: The Dictionary
│   │   ├── src/
│   │   │   └── lib.rs              # Re-exports generated data
│   │   ├── assets/
│   │   │   └── lexicon.json        # Vocabulary database (181KB)
│   │   ├── build.rs                # The 1,664-line generator
│   │   └── Cargo.toml              # deps: base + serde (build)
│   │
│   ├── logicaffeine_kernel/        # TIER 1: Type Theory (NO LEXICON!)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── term.rs             # CoC terms
│   │   │   ├── context.rs          # Type context
│   │   │   ├── type_checker.rs     # Bidirectional typing
│   │   │   ├── reduction.rs        # Normalization
│   │   │   ├── ring.rs             # Ring solver
│   │   │   ├── lia.rs              # Linear integer arithmetic
│   │   │   ├── omega.rs            # Omega test
│   │   │   ├── cc.rs               # Congruence closure
│   │   │   ├── simp.rs             # Simplification
│   │   │   ├── termination.rs      # Termination checking
│   │   │   ├── positivity.rs       # Positivity checking
│   │   │   ├── prelude.rs          # Built-in definitions
│   │   │   └── error.rs
│   │   └── Cargo.toml              # deps: base ONLY
│   │
│   ├── logicaffeine_data/          # TIER 1: Pure Data (WASM-safe)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── types.rs            # Nat, Int, Real, Text, Bool, Char, Byte
│   │   │   ├── value.rs            # Value enum
│   │   │   ├── seq.rs              # Sequence type
│   │   │   ├── map.rs              # Map type
│   │   │   ├── set.rs              # Set type
│   │   │   ├── tuple.rs            # Tuple types
│   │   │   ├── indexing.rs         # Polymorphic indexing
│   │   │   └── crdt/               # 20+ CRDT implementations
│   │   │       ├── mod.rs
│   │   │       ├── gcounter.rs
│   │   │       ├── pncounter.rs
│   │   │       ├── lww.rs
│   │   │       ├── mvregister.rs
│   │   │       ├── orset.rs
│   │   │       ├── ormap.rs
│   │   │       ├── causal/         # Vector clocks, dots, contexts
│   │   │       └── sequence/       # RGA, YATA
│   │   └── Cargo.toml              # deps: base ONLY
│   │
│   ├── logicaffeine_system/        # TIER 1: Platform IO (Heavy)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── io.rs               # println, read_line
│   │   │   ├── fs/                 # VFS abstraction
│   │   │   │   ├── mod.rs
│   │   │   │   └── opfs.rs         # WASM OPFS
│   │   │   ├── file.rs             # Native file I/O
│   │   │   ├── storage/            # Persistent storage
│   │   │   ├── network/            # libp2p networking
│   │   │   │   ├── mod.rs
│   │   │   │   ├── behaviour.rs
│   │   │   │   ├── gossip.rs
│   │   │   │   ├── mesh.rs
│   │   │   │   └── protocol.rs
│   │   │   ├── distributed.rs      # Distributed<T>
│   │   │   ├── concurrency.rs      # Go-like channels
│   │   │   ├── time.rs
│   │   │   ├── random.rs
│   │   │   ├── env.rs
│   │   │   └── memory.rs           # Memory zones
│   │   └── Cargo.toml              # deps: base, data + tokio/libp2p
│   │
│   ├── logicaffeine_language/      # TIER 2: NL→FOL Pipeline
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── lexer.rs            # Tokenization (85KB)
│   │   │   ├── parser/             # 10 submodules
│   │   │   │   ├── mod.rs
│   │   │   │   ├── clause.rs
│   │   │   │   ├── common.rs
│   │   │   │   ├── modal.rs
│   │   │   │   ├── noun.rs
│   │   │   │   ├── pragmatics.rs
│   │   │   │   ├── quantifier.rs
│   │   │   │   ├── question.rs
│   │   │   │   ├── verb.rs
│   │   │   │   └── tests.rs
│   │   │   ├── ast/                # 4 modules
│   │   │   │   ├── mod.rs
│   │   │   │   ├── logic.rs
│   │   │   │   ├── stmt.rs
│   │   │   │   └── theorem.rs
│   │   │   ├── transpile.rs        # Unicode/LaTeX/SimpleFOL output
│   │   │   ├── semantics/          # 3 modules
│   │   │   │   ├── mod.rs
│   │   │   │   ├── axioms.rs
│   │   │   │   └── kripke.rs
│   │   │   ├── lambda.rs           # Lambda calculus (65KB)
│   │   │   ├── session.rs          # Incremental REPL
│   │   │   ├── mwe.rs              # Multi-word expressions
│   │   │   ├── ontology.rs         # Ontology system
│   │   │   ├── pragmatics.rs       # Pragmatics analysis
│   │   │   ├── runtime_lexicon.rs  # Runtime vocabulary
│   │   │   ├── scope.rs            # Scope management
│   │   │   ├── symbol_dict.rs      # Symbol dictionary
│   │   │   ├── suggest.rs          # Suggestion engine
│   │   │   ├── view.rs             # Expression views
│   │   │   ├── visitor.rs          # AST visitors
│   │   │   ├── arena_ctx.rs        # Context bundling
│   │   │   ├── formatter.rs        # Output formatting (25KB)
│   │   │   ├── debug.rs            # Debugging utilities
│   │   │   └── error.rs
│   │   └── Cargo.toml              # deps: base, lexicon
│   │
│   ├── logicaffeine_proof/         # TIER 2: Proof Engine
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── engine.rs           # Backward chaining
│   │   │   ├── convert.rs          # LogicExpr → ProofExpr
│   │   │   ├── unify.rs            # Unification algorithm
│   │   │   ├── certifier.rs        # Proof verification
│   │   │   ├── hints.rs            # Socratic hints
│   │   │   └── error.rs
│   │   └── Cargo.toml              # deps: base, kernel, language
│   │
│   ├── logicaffeine_compile/       # TIER 2: LOGOS Compilation
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── compile.rs          # Pipeline orchestrator
│   │   │   ├── codegen.rs          # Rust code generation
│   │   │   ├── interpreter.rs      # Tree-walking runtime
│   │   │   ├── analysis/           # 7 static analysis passes
│   │   │   │   ├── registry.rs     # Type definitions
│   │   │   │   ├── discovery.rs    # Type discovery
│   │   │   │   ├── dependencies.rs # Dependency scanning
│   │   │   │   ├── escape.rs       # Escape analysis
│   │   │   │   ├── ownership.rs    # Ownership checking
│   │   │   │   └── policy.rs       # Capability policies
│   │   │   └── extraction/         # Program extraction
│   │   └── Cargo.toml              # deps: language, data, system
│   │
│   └── logicaffeine_verify/        # TIER 3: Z3 Verification (Optional)
│       ├── src/
│       │   ├── lib.rs
│       │   ├── solver.rs           # Z3 wrapper
│       │   ├── ir.rs               # Lightweight IR
│       │   ├── license.rs          # Stripe validation
│       │   └── error.rs
│       └── Cargo.toml              # deps: base, kernel, z3
│
└── apps/
    ├── logicaffeine_cli/           # TIER 4: CLI Tool (largo)
    │   ├── src/
    │   │   ├── main.rs
    │   │   └── project/            # Multi-file project system
    │   └── Cargo.toml              # deps: all crates + clap
    │
    └── logicaffeine_web/           # TIER 4: Web UI
        ├── src/
        │   ├── main.rs
        │   └── ui/                 # Dioxus components
        └── Cargo.toml              # deps: language, proof, compile + dioxus
```

## 3.1 Workspace Cargo.toml Configuration

```toml
[workspace]
resolver = "2"
members = [
    "crates/logicaffeine_base",
    "crates/logicaffeine_lexicon",
    "crates/logicaffeine_kernel",
    "crates/logicaffeine_data",
    "crates/logicaffeine_system",
    "crates/logicaffeine_language",
    "crates/logicaffeine_proof",
    "crates/logicaffeine_compile",
    "apps/logicaffeine_cli",
    "apps/logicaffeine_web",
]
# CRITICAL: Exclude verify from default builds (requires Z3 C++ libs)
exclude = ["crates/logicaffeine_verify"]

[workspace.dependencies]
# Shared versions for consistency
bumpalo = "3.19"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# KNUTH DOCUMENTATION STANDARD (Section 1.8)
[workspace.metadata.docs.rs]
all-features = false                    # Don't include verify feature (requires Z3)
features = ["cli"]                      # Include CLI documentation
cargo-args = ["--workspace"]            # Build all crates together
rustdoc-args = ["--document-private-items"]  # Document internals for implementors
```

**Z3 Exclusion Rationale**: The `logicaffeine_verify` crate depends on `z3-sys` which requires Z3 C++ libraries installed on the build machine. Excluding it from the workspace ensures:
- `cargo build --workspace` works on any machine
- `cargo test --workspace` doesn't require Z3
- Verification is opt-in via `cargo build -p logicaffeine_verify`
- CI pipelines work without special Z3 setup

## 3.5 Complete Module-to-Crate Mapping

### logicaffeine_base (6 modules)
| Module | Purpose |
|--------|---------|
| `arena.rs` | Bump allocation wrapper |
| `intern.rs` | Symbol interning (extracted from lib.rs) |
| `span.rs` | Source location tracking |
| `token.rs` | Token types (extracted from lexer.rs) |
| `error.rs` | Base error types |
| `lib.rs` | Exports |

### logicaffeine_lexicon (generated)
| Component | Purpose |
|-----------|---------|
| `build.rs` | 1,664-line lexicon generator |
| `lexicon.json` | Vocabulary database (181KB) |
| `lexicon_data.rs` | Generated lookup functions (40+) |
| `mwe_data.rs` | Multi-word expressions |
| `ontology_data.rs` | Part-whole relations |
| `axiom_data.rs` | Meaning postulates |

### logicaffeine_kernel (14 modules)
| Module | Purpose |
|--------|---------|
| `term.rs` | CoC terms |
| `context.rs` | Type context |
| `type_checker.rs` | Bidirectional typing |
| `reduction.rs` | Normalization (β, η, ι) |
| `ring.rs` | Ring solver tactic |
| `lia.rs` | Linear integer arithmetic |
| `omega.rs` | Omega test |
| `cc.rs` | Congruence closure |
| `simp.rs` | Simplification |
| `termination.rs` | Termination checking |
| `positivity.rs` | Positivity checking |
| `prelude.rs` | Built-in definitions |
| `error.rs` | Kernel errors |

### logicaffeine_language (27 modules)
| Module | Purpose |
|--------|---------|
| `lexer.rs` | Tokenization |
| `parser/*.rs` | 9 submodules: clause, common, modal, noun, pragmatics, quantifier, question, verb, tests |
| `ast/*.rs` | 4 submodules: logic, stmt, theorem, mod |
| `transpile.rs` | Unicode/LaTeX/SimpleFOL output |
| `semantics/*.rs` | 3 submodules: axioms, kripke, mod |
| `lambda.rs` | Lambda calculus support |
| `session.rs` | Incremental REPL evaluation |
| `mwe.rs` | Multi-word expressions |
| `ontology.rs` | Ontology system |
| `pragmatics.rs` | Pragmatics analysis |
| `runtime_lexicon.rs` | Runtime vocabulary |
| `scope.rs` | Scope management |
| `symbol_dict.rs` | Symbol dictionary |
| `suggest.rs` | Suggestion engine |
| `view.rs` | Expression views |
| `visitor.rs` | AST visitors |
| `arena_ctx.rs` | Context bundling |
| `formatter.rs` | Output formatting |
| `debug.rs` | Debugging utilities |
| `drs.rs` | Discourse Representation Structures |
| `error.rs` | Language errors |

### logicaffeine_data (WASM-safe, ~25 modules)
| Module | Purpose |
|--------|---------|
| `types.rs` | Nat, Int, Real, Text, Bool, Unit, Char, Byte |
| `value.rs` | Value enum |
| `seq.rs` | Sequence type |
| `map.rs` | Map type |
| `set.rs` | Set type |
| `tuple.rs` | Tuple types |
| `indexing.rs` | Polymorphic indexing |
| `crdt/*.rs` | 20+ files: gcounter, pncounter, lww, mvregister, orset, ormap |
| `crdt/causal/*.rs` | Vector clocks, dots, contexts |
| `crdt/sequence/*.rs` | RGA, YATA |

### logicaffeine_system (platform IO, ~16 modules)
| Module | Purpose |
|--------|---------|
| `io.rs` | println, show, read_line |
| `fs/*.rs` | VFS abstraction: mod, opfs |
| `file.rs` | Native file I/O |
| `storage/*.rs` | Persistent storage |
| `network/*.rs` | 8 files: behaviour, gossip, mesh, protocol, sipping, wire |
| `distributed.rs` | Distributed<T> unified persistence |
| `concurrency.rs` | Go-like channels (native only) |
| `time.rs` | Time primitives (native only) |
| `random.rs` | RNG (native only) |
| `env.rs` | Environment variables (native only) |
| `memory.rs` | Memory zones (native only) |

### logicaffeine_proof (8 modules)
| Module | Purpose |
|--------|---------|
| `engine.rs` | Backward chaining |
| `convert.rs` | LogicExpr → ProofExpr |
| `unify.rs` | Unification algorithm |
| `certifier.rs` | Proof verification |
| `hints.rs` | Socratic hints |
| `oracle.rs` | External solver integration |
| `error.rs` | Proof errors |

### logicaffeine_compile (17 modules)
| Module | Purpose |
|--------|---------|
| `compile.rs` | Pipeline orchestrator (native-only) |
| `codegen.rs` | Rust code generation |
| `interpreter.rs` | Tree-walking runtime |
| `diagnostic.rs` | Ownership error translation (native-only) |
| `sourcemap.rs` | Source mapping for errors (native-only) |
| `analysis/*.rs` | 7 files: registry, discovery, dependencies, escape, ownership, policy, mod |
| `extraction/*.rs` | 4 files: codegen, collector, error, mod |

### logicaffeine_verify (5 modules)
| Module | Purpose |
|--------|---------|
| `solver.rs` | Z3 wrapper |
| `ir.rs` | Lightweight IR (VerifyExpr, VerifyOp, VerifyType) |
| `license.rs` | Stripe validation |
| `error.rs` | Verification errors |
| `verification.rs` | Integration point from main crate |

### logicaffeine_cli (15 modules)
| Module | Purpose |
|--------|---------|
| `cli.rs` | CLI implementation |
| `main.rs` | CLI dispatch entry |
| `interface/*.rs` | 6 files: command, command_parser, error, literate_parser, repl, term_parser |
| `project/*.rs` | 6 files: build, credentials, loader, manifest, registry, mod |
| `registry.rs` | Package registry client |

### Workspace-Level Test Utilities
| Module | Purpose |
|--------|---------|
| `test_utils.rs` | Shared test utilities (arena creation, parsing helpers) |

These remain at the workspace root or in a `logicaffeine_test_utils` dev-only crate.

### logicaffeine_web (52 modules)
| Module | Purpose |
|--------|---------|
| `ui/app.rs` | Root app component |
| `ui/router.rs` | Route definitions |
| `ui/state.rs` | App state management |
| `ui/theme.rs` | Theme configuration |
| `ui/responsive.rs` | Responsive utilities |
| `ui/examples.rs` | Example sentences |
| `ui/components/*.rs` | 30+ components (see below) |
| `ui/hooks/*.rs` | Custom hooks |
| `ui/pages/*.rs` | 15+ route pages |
| `achievements.rs` | Gamification tracking |
| `audio.rs` | Audio synthesis |
| `content.rs` | Embedded curriculum content |
| `game.rs` | Game mechanics |
| `generator.rs` | Problem generation |
| `grader.rs` | Exercise grading |
| `learn_state.rs` | Tab/focus state for learning UI |
| `progress.rs` | User progress tracking |
| `srs.rs` | Spaced repetition |
| `storage.rs` | Browser storage (WASM-only) |
| `struggle.rs` | Difficulty tracking |
| `style.rs` | Styling utilities |
| `unlock.rs` | Feature unlocking |

**UI Components (30+):**
achievement_toast, app_navbar, ast_tree, chat, code_editor, combo_indicator, context_view, editor, file_browser, formula_editor, guide_code_block, guide_sidebar, input, katex, learn_sidebar, logic_output, main_nav, mixed_text, mode_selector, mode_toggle, module_tabs, proof_panel, repl_output, socratic_guide, streak_display, symbol_dictionary, symbol_palette, vocab_reference, xp_popup

**UI Pages (15+):**
guide/, registry/ (browse, package_detail), landing, learn, lesson, pricing, privacy, profile, review, roadmap, studio, success, terms, workspace

## 4. Dependency Graph

```
                         logicaffeine_base
                                │
            ┌───────────────────┼───────────────────┐
            │                   │                   │
            ▼                   ▼                   ▼
   logicaffeine_lexicon  logicaffeine_kernel  logicaffeine_data
            │                   │                   │
            │                   ▼                   │
            │          logicaffeine_proof           │
            │           (KERNEL ONLY!)              │
            │                   │                   │
            └─────────┬─────────┘                   │
                      ▼                             ▼
            logicaffeine_language         logicaffeine_system
            (lexicon + proof)                       │
                      │                             │
                      └──────────────┬──────────────┘
                                     ▼
                          logicaffeine_compile
                          (language + data + system)
                                     │
                    ┌────────────────┴────────────────┐
                    ▼                                 ▼
            logicaffeine_cli                  logicaffeine_web


    ╔════════════════════════════════════════════════════════╗
    ║  EXCLUDED FROM WORKSPACE (opt-in only via --features)  ║
    ╠════════════════════════════════════════════════════════╣
    ║                                                        ║
    ║    logicaffeine_verify ──► base + kernel + z3          ║
    ║                                                        ║
    ╚════════════════════════════════════════════════════════╝
```

**Key Architectural Properties (Liskov Compliant):**
1. The kernel (pure math) has NO path to lexicon (English words)
2. The proof crate depends on kernel ONLY (Liskov Trait: no language dependency)
3. The language crate depends on proof (provides EnglishHintRenderer)
4. The verify crate is EXCLUDED from default workspace builds
5. The data crate (CRDTs) has NO path to system (networking)

## 5. User Scenarios

| Use Case | Crates Required | Approx Size |
|----------|-----------------|-------------|
| FOL parsing only | base + lexicon + language | ~315 KB |
| Type theory only | base + kernel | ~65 KB |
| CRDTs only (WASM) | base + data | ~120 KB |
| Proof search | base + lexicon + kernel + language + proof | ~395 KB |
| LOGOS compilation | base + lexicon + language + data + system + compile | ~565 KB |
| Full CLI | all crates | ~800 KB |

## 6. Implementation Roadmap: The Golden Path

The Council has ordained four named phases:

### The Golden Path Overview

| Phase | Name | Goal | Litmus Test |
|-------|------|------|-------------|
| 1 | **Milner** | Core Separation | `grep "lexicon" crates/logicaffeine_kernel` returns ZERO |
| 2 | **Lamport** | Data Liberation | `cargo build -p logicaffeine_data --target wasm32-unknown-unknown` succeeds |
| 3 | **Hopper** | Reconstruction | All crates compile with correct dependencies |
| 4 | **Ballmer** | Applications | `cargo test --workspace -- --skip e2e` passes |

---

## 6.0 The Linus Protocol: Preserving Git History
**Status: MANDATORY (Torvalds/Primeagen Mandate)**

> "Talk is cheap. Show me the code. If you copy-paste, you destroy the history.
> `git blame` becomes useless. I will not accept a PR where I cannot see who
> wrote the code 3 years ago." — Linus Torvalds

### The Golden Rule

**ALWAYS use `git mv` to move files.** Never copy-paste then delete.

### Execution Protocol

```bash
# 1. Safety First: Create a branch
git checkout -b refactor/phase-N-<crate-name>

# 2. Create the destination directory structure
mkdir -p crates/logicaffeine_<name>/src

# 3. Move files with git mv (PRESERVES HISTORY)
git mv src/kernel crates/logicaffeine_kernel/src/

# 4. Create the Cargo.toml for the new crate AFTER the move
# (Don't create it before, or git gets confused)

# 5. Fix imports in moved files
# - Change `crate::` to appropriate paths
# - Add `use` statements for cross-crate deps

# 6. Fix imports in files that referenced the moved code
# - Update paths to use the new crate

# 7. Verify green state
cargo test -- --skip e2e

# 8. Commit with clear message
git commit -m "Extract logicaffeine_<name> crate

- Moved X files using git mv
- Updated Y import paths
- All tests pass"
```

### Why This Matters

| Method | `git blame` | `git log --follow` | History |
|--------|-------------|-------------------|---------|
| `git mv` | ✅ Works | ✅ Works | Preserved |
| Copy + Delete | ❌ Broken | ❌ Broken | Lost |
| IDE "Move" | ⚠️ Maybe | ⚠️ Maybe | Depends |

### Verification

After each move, verify history is preserved:

```bash
# Check that git tracks the rename
git log --follow --oneline crates/logicaffeine_kernel/src/mod.rs
# Should show commits from BEFORE the move

# Check git blame works
git blame crates/logicaffeine_kernel/src/reduction.rs | head -5
# Should show original authors, not just "refactor commit"
```

### One Crate At A Time

**DO NOT** move all files at once. The protocol is:

1. Move ONE crate's files
2. Fix imports
3. Verify tests pass
4. Commit
5. Repeat for next crate

This ensures if something breaks, you know exactly which move caused it.

---

## 6.1 TDD Protocol for Refactoring

**From CLAUDE.md**: "If a test is failing it is ALWAYS A REGRESSION. We do not move forward until ALL TESTS PASS, and we START FROM A POINT OF ALL TESTS PASSING."

Every phase follows strict TDD discipline. This section defines the protocol.

### Pre-Phase Checklist

Before starting ANY phase:

```bash
# 1. Verify green baseline
cargo test -- --skip e2e
# Expected: 0 failures, ~2,500 tests

# 2. Document current test count
cargo test -- --skip e2e 2>&1 | grep "test result"
# Record: "test result: ok. XXX passed; 0 failed"

# 3. Create phase branch
git checkout -b phase-N-<name>
```

### Per-Crate Extraction Protocol

For each crate extraction, follow RED → GREEN → CHECKPOINT:

#### Step 1: BASELINE (Verify Green)

```bash
cargo test -- --skip e2e
# MUST show: 0 failures
# If ANY test fails: STOP. Fix before proceeding.
```

#### Step 2: WRITE RED TEST (Spec First)

Create a test that defines success for this extraction:

```rust
// tests/phase_N_<crate>_extraction.rs

/// This test defines the extraction success criteria.
/// It will be RED until the extraction is complete.
#[test]
fn <crate>_isolation_litmus_test() {
    // Example for kernel:
    // Verify: cargo tree -p logicaffeine_kernel | grep lexicon
    // Must return: EMPTY (no lexicon dependency)
}
```

#### Step 3: EXTRACT (Make Green)

Perform the extraction work:
1. Create the new crate directory
2. Move files from monolith to new crate
3. Update imports in moved files
4. Update imports in remaining files that referenced moved code
5. Fix any compilation errors

**CRITICAL**: If you encounter a failing test during extraction:
- **DO NOT modify the test** (the test is the spec)
- Fix the implementation to satisfy the test
- If the test seems wrong, STOP and ask the user

#### Step 4: VERIFY GREEN

```bash
# All existing tests must pass
cargo test -- --skip e2e
# MUST show: 0 failures

# New extraction test must pass
cargo test <crate>_isolation_litmus_test
# MUST show: 1 passed

# Crate-specific verification
cargo build -p logicaffeine_<crate>
# MUST succeed
```

#### Step 5: CHECKPOINT (Commit)

```bash
# Commit with clear message
git add .
git commit -m "Extract logicaffeine_<crate> - all tests pass

- Moved X files to crates/logicaffeine_<crate>/
- Updated Y import paths
- Verified: cargo test -- --skip e2e passes
- Verified: <litmus test> passes"
```

### Phase Completion Criteria

A phase is complete ONLY when ALL of the following are true:

| Criterion | Verification Command | Expected Result |
|-----------|---------------------|-----------------|
| All tests pass | `cargo test -- --skip e2e` | 0 failures |
| Workspace builds | `cargo build --workspace` | Success |
| Phase litmus test | (varies by phase) | As specified |
| No new warnings | `cargo build --workspace 2>&1 \| grep warning` | Empty or unchanged |
| Test count stable | Compare to baseline | Same or higher |

### Test Count Tracking

Maintain a running log during refactoring:

| Checkpoint | Test Files | Tests Run | Tests Passed | Status |
|------------|-----------|-----------|--------------|--------|
| Pre-start baseline | 222 | ~2,536 | ~2,536 | ✓ |
| After Phase 1.1 (base) | 222 | ~2,536 | ~2,536 | |
| After Phase 1.2 (lexicon) | 222 | ~2,536 | ~2,536 | |
| After Phase 1.3 (kernel) | 222 | ~2,536 | ~2,536 | |
| After Phase 2.1 (data) | 222 | ~2,536 | ~2,536 | |
| After Phase 3.1 (system) | 222 | ~2,536 | ~2,536 | |
| After Phase 3.2 (language) | 222 | ~2,536 | ~2,536 | |
| After Phase 3.3 (proof) | 222 | ~2,536 | ~2,536 | |
| After Phase 3.4 (compile) | 222 | ~2,536 | ~2,536 | |
| After Phase 4 (apps) | 222 | ~2,536 | ~2,536 | |
| Final | 222+ | ~2,536+ | ~2,536+ | |

### Recovery from Red Tests

If a test fails during refactoring:

1. **STOP immediately** - Do not continue with more changes
2. **DO NOT modify the failing test** - The test defines the spec
3. **Revert to last green state**:
   ```bash
   git stash  # or git checkout -- .
   ```
4. **Analyze the failure**:
   - What import path broke?
   - What re-export is missing?
   - What circular dependency was introduced?
5. **Try smaller incremental changes**
6. **If stuck**: Ask the user for clarification before proceeding

### E2E Test Protocol

E2E tests are skipped during extraction phases for speed:

```bash
# Default during extraction (fast, ~30 seconds)
cargo test -- --skip e2e

# Full validation before PR (slow, ~5 minutes)
cargo test

# With verification if Z3 available
cargo test --features verification
```

E2E tests compile LOGOS to Rust binaries and execute them. They test:
- Runtime behavior of compiled programs
- Cross-crate integration at the binary level
- Platform-specific features (native only)

Run full E2E tests at these milestones:
- End of Phase 2 (Data Liberation)
- End of Phase 3 (Reconstruction)
- End of Phase 4 (Applications)

---

### Phase 1: The Milner Phase — Workspace Infrastructure
1. Initialize workspace `Cargo.toml`
2. Create `crates/` and `apps/` directories
3. Scaffold empty crate manifests

### Phase 1.1: Base Extraction (Tier 0)

**TDD BASELINE**: `cargo test -- --skip e2e` must pass before starting.

**Steps**:
1. Create `crates/logicaffeine_base/`
2. Extract `arena.rs` (direct move)
3. Extract `Interner` struct from `src/lib.rs` → `intern.rs`
4. Extract `Span` types from `src/error.rs` → `span.rs`
5. Extract `TokenKind` enum from `src/lexer.rs` → `token.rs`
6. Create minimal `error.rs` (base error types only)
7. Wire up `lib.rs` exports

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_base                    # Must succeed
cargo tree -p logicaffeine_base                     # Only shows bumpalo
cargo test -- --skip e2e                            # 0 failures (all tests pass)
```

### Phase 1.2: Lexicon Extraction (Tier 0.5)

**TDD BASELINE**: Phase 1.1 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_lexicon/`
2. Copy `build.rs` (1,664-line generator)
3. Move `assets/lexicon.json` to `crates/logicaffeine_lexicon/assets/`
4. Move `src/runtime_lexicon.rs` to `crates/logicaffeine_lexicon/src/runtime.rs`
5. Update `build.rs` paths for new location
6. Create `lib.rs` with feature-gated exports:
   ```rust
   // Default: use generated static code
   #[cfg(not(feature = "dynamic-lexicon"))]
   include!(concat!(env!("OUT_DIR"), "/lexicon_data.rs"));

   // Alternative: runtime JSON loading (faster compiles)
   #[cfg(feature = "dynamic-lexicon")]
   mod runtime;
   #[cfg(feature = "dynamic-lexicon")]
   pub use runtime::*;
   ```
7. Create `Cargo.toml`:
   ```toml
   [package]
   name = "logicaffeine-lexicon"
   version = "0.1.0"
   edition = "2021"

   [dependencies]
   logicaffeine-base = { path = "../logicaffeine_base" }

   # Only needed for dynamic-lexicon feature
   serde = { version = "1.0", features = ["derive"], optional = true }
   serde_json = { version = "1.0", optional = true }
   rand = { version = "0.8", optional = true }

   [build-dependencies]
   serde = { version = "1.0", features = ["derive"] }
   serde_json = "1.0"

   [features]
   default = []
   dynamic-lexicon = ["serde", "serde_json", "rand"]
   ```

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine-lexicon                              # Static build succeeds
cargo build -p logicaffeine-lexicon --features dynamic-lexicon   # Dynamic build succeeds
find target -name "lexicon_data.rs" -exec wc -l {} \;            # 10,000+ lines generated
cargo test -- --skip e2e                                         # 0 failures (all tests pass)
```

### Phase 1.3: Kernel Extraction (Tier 1) — THE MILNER LITMUS TEST

**TDD BASELINE**: Phase 1.2 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_kernel/`
2. Move all `src/kernel/*.rs` files
3. Add dependency on `base` ONLY (NOT lexicon!)
4. Audit imports: grep for "lexer", "parser", "lexicon" - must find ZERO

**TDD CHECKPOINT** (THE MILNER TEST):
```bash
cargo build -p logicaffeine_kernel                                    # Must succeed
grep -r "lexicon\|lexer\|parser" crates/logicaffeine_kernel/src/      # Must return EMPTY
cargo tree -p logicaffeine_kernel | grep lexicon                      # Must return EMPTY
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

**CRITICAL**: If kernel has ANY path to lexicon, STOP and refactor. The kernel must be pure math.

---

### Phase 2: The Lamport Phase — Data Liberation

### Phase 2.1: Data Extraction (Tier 1)

**TDD BASELINE**: Phase 1.3 checkpoint must pass.

**Pre-Extraction Fix (LWW Time Injection)**:
1. Refactor `logos_core/src/crdt/lww.rs`:
   - Remove `use std::time::{SystemTime, UNIX_EPOCH};`
   - Change `new(value: T)` to `new(value: T, timestamp: u64)`
   - Change `set(&mut self, value: T)` to `set(&mut self, value: T, timestamp: u64)`
   - Remove `fn now() -> u64` helper
2. Update all callers to provide timestamps explicitly
3. Verify: `cargo test -- --skip e2e` passes with new API

**Steps**:
1. Create `crates/logicaffeine_data/`
2. Move from `logos_core/`: `types.rs`, `crdt/*.rs`, `indexing.rs`
3. Add dependency on `base` ONLY

**TDD CHECKPOINT** (THE LAMPORT TEST):
```bash
cargo build -p logicaffeine_data                                      # Native build succeeds
cargo build -p logicaffeine_data --target wasm32-unknown-unknown      # WASM build succeeds
cargo tree -p logicaffeine_data | grep -E "tokio|libp2p"              # Must return EMPTY (no IO deps)
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

**MILESTONE**: Run full E2E tests after this phase:
```bash
cargo test  # Full test suite including E2E
```

### Phase 3: The Hopper Phase — Reconstruction

### Phase 3.1: System Extraction (Tier 1) — CERF/DRASNER FEATURE FLAGS

**TDD BASELINE**: Phase 2.1 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_system/`
2. Move from `logos_core/`: `io.rs`, `fs/*.rs`, `network/*.rs`, `distributed.rs`
3. Move: `concurrency.rs`, `file.rs`, `time.rs`, `random.rs`, `env.rs`, `memory.rs`
4. Add dependencies: `base`, `data`
5. Configure platform-specific deps in Cargo.toml
6. **Implement Cerf/Drasner feature flags** (see Section 1.7):
   - `networking` (libp2p)
   - `persistence` (memmap2, sha2)
   - `concurrency` (rayon)

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_system                                    # Minimal build (no features)
cargo build -p logicaffeine_system --features networking              # With networking
cargo build -p logicaffeine_system --features persistence             # With persistence
cargo build -p logicaffeine_system --features full                    # With all features
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

### Phase 3.2: Language Extraction (Tier 2)

**TDD BASELINE**: Phase 3.1 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_language/`
2. Move: `lexer.rs`, `parser/*`, `ast/*`, `transpile.rs`, `semantics/*`
3. Move: `lambda.rs`, `session.rs`, `mwe.rs`, `ontology.rs`, `pragmatics.rs`
4. Move: `scope.rs`, `symbol_dict.rs`, `suggest.rs`, `view.rs`, `visitor.rs`
5. Move: `arena_ctx.rs`, `formatter.rs`, `debug.rs`, `runtime_lexicon.rs`
6. Add dependencies: `base`, `lexicon`

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_language                                  # Must succeed
cargo test -p logicaffeine_language                                   # Language tests pass
# Smoke test: Parse "Socrates is a man"
cargo test phase1_garden_path                                         # Phase 1 tests pass
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

### Phase 3.3: Proof Extraction (Tier 2) — LISKOV TRAIT

**TDD BASELINE**: Phase 3.2 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_proof/`
2. Move `src/proof/*.rs`
3. **Implement Liskov Trait** (see Section 1.6):
   - Define `HintRenderer` trait in proof crate
   - Implement `DefaultHintRenderer` in proof crate (self-contained, no language dep)
   - Remove ALL imports from `language` crate in proof module
4. Add dependencies: `kernel` ONLY (proof has ZERO dependency on language)
5. Update `language` crate to depend on `proof` and implement `EnglishHintRenderer`

**Dependency Inversion**: The proof crate exports `HintRenderer`. The language crate IMPORTS it and provides `EnglishHintRenderer`. CLI/Web wire them together. This is the Liskov Substitution Principle in action.

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_proof                                     # Must succeed
cargo tree -p logicaffeine_proof | grep language                      # Must return EMPTY (not optional - ZERO)
cargo build -p logicaffeine_language                                  # Must succeed (depends on proof)
cargo test phase35                                                    # Proof tests pass
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

### Phase 3.4: Compile Extraction (Tier 3)

**TDD BASELINE**: Phase 3.3 checkpoint must pass.

**Steps**:
1. Create `crates/logicaffeine_compile/`
2. Move: `codegen.rs`, `interpreter.rs`, `compile.rs`, `diagnostic.rs`, `sourcemap.rs`
3. Move: `analysis/*`, `extraction/*`
4. Move: `assets/std/*` to `crates/logicaffeine_compile/assets/std/`
5. Add dependencies: `language`, `data`, `system`
6. Add feature flags: `codegen`, `interpreter-only`

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_compile                                   # Must succeed
cargo build -p logicaffeine_compile --features interpreter-only       # Minimal build
cargo test phase21                                                    # Compile tests pass
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

**MILESTONE**: Run full E2E tests after this phase:
```bash
cargo test  # Full test suite including E2E - tests compilation pipeline
```

### Phase 3.5: Verify Migration (Tier 3.5)

**TDD BASELINE**: Phase 3.4 checkpoint must pass.

**Steps**:
1. Rename `logos_verification/` → `crates/logicaffeine_verify/`
2. Update dependencies to new crate names
3. Keep excluded from workspace (requires Z3)

**TDD CHECKPOINT** (requires Z3 installed):
```bash
cargo build -p logicaffeine_verify                                    # Must succeed (if Z3 available)
cargo test --features verification phase_verification                 # Verification tests pass
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

### Phase 4: The Ballmer Phase — Applications

### Phase 4.1: Apps Migration (Tier 4)

**TDD BASELINE**: Phase 3.5 checkpoint must pass.

**Steps**:
1. Create `apps/logicaffeine_cli/`
   - Move: `cli.rs`, `interface/*`, `project/*`
   - Update all imports
   - Wire up `EnglishHintRenderer` for rich proof hints
2. Create `apps/logicaffeine_web/`
   - Move: `ui/*`
   - Move gamification: `achievements.rs`, `audio.rs`, `content.rs`, `game.rs`, `generator.rs`, `grader.rs`, `learn_state.rs`, `progress.rs`, `srs.rs`, `storage.rs`, `struggle.rs`, `style.rs`, `unlock.rs`
   - Move: `assets/curriculum/`, images
   - Update: `Dioxus.toml`
   - Wire up `EnglishHintRenderer` for rich proof hints

**TDD CHECKPOINT**:
```bash
cargo build -p logicaffeine_cli                                       # CLI builds
cargo build -p logicaffeine_web --target wasm32-unknown-unknown       # Web builds for WASM
cargo test -- --skip e2e                                              # 0 failures (all tests pass)
```

### Phase 4.2: CI/CD Updates

**TDD BASELINE**: Phase 4.1 checkpoint must pass.

**Steps**:
1. Change `ubuntu-latest-m` to `ubuntu-latest` in `.github/workflows/test.yml`
2. Update `test.yml` to `cargo test --workspace`
3. Update `deploy-frontend.yml` to build from `apps/logicaffeine_web/`
4. Update deploy path
5. Add docs workflow (Knuth Documentation Standard)

**TDD CHECKPOINT**:
```bash
# Local verification before pushing
cargo test --workspace -- --skip e2e                                  # All workspace tests pass
cargo doc --workspace --no-deps --exclude logicaffeine_verify         # Docs build
# After push: All GitHub Actions workflows green
```

### Phase 4.3: Final Verification — THE BALLMER TEST

**This is the final gate. ALL items must pass before the refactor is complete.**

**Architectural Invariants**:
```bash
# Milner: Kernel has NO lexicon dependency
cargo tree -p logicaffeine_kernel | grep lexicon                      # MUST return EMPTY

# Liskov: Proof has NO language dependency
cargo tree -p logicaffeine_proof | grep language                      # MUST return EMPTY

# Lamport: Data crate is WASM-safe
cargo build -p logicaffeine_data --target wasm32-unknown-unknown      # MUST succeed

# Cerf/Drasner: System builds with no features
cargo build -p logicaffeine_system                                    # MUST succeed (lean)
```

**Functional Tests**:
```bash
# All tests pass
cargo test --workspace -- --skip e2e                                  # ~2,536 tests, 0 failures

# Full E2E (compilation pipeline)
cargo test --workspace                                                # Including E2E tests

# Verification (if Z3 available)
cargo test --features verification                                    # Z3 tests pass
```

**Application Builds**:
```bash
cargo build -p logicaffeine_cli --release                             # CLI binary
cargo build -p logicaffeine_web --target wasm32-unknown-unknown       # WASM bundle
dx build --release                                                    # Full web app
```

**Manual Verification**:
- [ ] Web app loads in browser at localhost
- [ ] `largo repl` starts and parses "Socrates is a man"
- [ ] Proof hints display correctly (EnglishHintRenderer working)
- [ ] All CI workflows green on GitHub

**MILESTONE**: Run FULL test suite one final time:
```bash
cargo test  # Everything. No skips. This is the Ballmer Test.
```

## 7. Deployment Strategy

### crates.io
| Crate | Package Name | Description |
|-------|--------------|-------------|
| base | `logicaffeine-base` | Pure atoms (arena, tokens, spans) |
| lexicon | `logicaffeine-lexicon` | English vocabulary database |
| kernel | `logicaffeine-kernel` | Calculus of Constructions type system |
| data | `logicaffeine-data` | WASM-safe data structures & CRDTs |
| system | `logicaffeine-system` | Platform IO (networking, persistence) |
| language | `logicaffeine-language` | English-to-Logic transpiler |
| proof | `logicaffeine-proof` | Backward-chaining proof engine |
| compile | `logicaffeine-compile` | LOGOS compilation pipeline |
| verify | `logicaffeine-verify` | Z3-based static verification |

### Homebrew (CLI)
- **Tap**: `brahmastra-labs/homebrew-tap`
- **Formula**: `logicaffeine.rb`
- **Binary**: `largo`

### Web (Wasm)
- **Host**: Cloudflare Pages
- **Build**: `dx bundle --release`

## 7.5 Platform Feature Matrix

### Conditional Compilation Requirements

These modules require platform-specific compilation flags:

| Module | Condition | Target Crate |
|--------|-----------|--------------|
| `compile.rs` | `#[cfg(not(target_arch = "wasm32"))]` | compile |
| `diagnostic.rs` | `#[cfg(not(target_arch = "wasm32"))]` | compile |
| `sourcemap.rs` | `#[cfg(not(target_arch = "wasm32"))]` | compile |
| `project/*` | `#[cfg(not(target_arch = "wasm32"))]` | cli |
| `cli.rs` | `#[cfg(all(not(wasm32), feature = "cli"))]` | cli |
| `verification.rs` | `#[cfg(all(not(wasm32), feature = "verification"))]` | verify |
| `storage.rs` | `#[cfg(target_arch = "wasm32")]` | web |

### Feature Flags

| Feature | Crates Affected | Dependencies Enabled |
|---------|-----------------|---------------------|
| `cli` | logicaffeine_cli | `clap`, `toml`, `ureq`, `flate2`, `tar`, `dirs` |
| `verification` | logicaffeine_verify | `logos_verification`, `z3` |

### System Feature Flags (Cerf/Drasner Amendment)

The `logicaffeine_system` crate uses feature flags for heavy dependencies (see Section 1.7):

| Feature | Dependencies | Modules Enabled | Use Case |
|---------|-------------|-----------------|----------|
| (default) | async-trait, once_cell | `io`, `time`, `env`, `random` | Simple scripts, minimal size |
| `networking` | libp2p, futures | `network/*` | P2P applications |
| `persistence` | memmap2, sha2 | `storage/*`, `fs/*`, `file` | File processing |
| `concurrency` | rayon | `concurrency`, `memory` | Parallel computation |
| `distributed` | networking + persistence | `distributed` | Collaborative apps |
| `full` | All of above | All modules | Full server |

**Default is LEAN**: `cargo build -p logicaffeine_system` produces ~50KB binary with no networking stack.

### Lexicon Feature Flags

| Feature | Build Behavior | Runtime Behavior | Use Case |
|---------|---------------|------------------|----------|
| (default) | `build.rs` generates static match expressions | O(1) lookups, zero allocation | Production, WASM, CI |
| `dynamic-lexicon` | `build.rs` skipped | JSON parsed at startup | Development, fast iteration |

**Recommendation**: Use default for CI/release. Use `dynamic-lexicon` during active lexicon development.

---

## 8. Migration Checklist

### Pre-Migration
- [ ] Tag current state: `git tag pre-packaging`
- [ ] Create feature branch: `git checkout -b feature/tiered-crates`
- [ ] Document current test count: 222 files, X passing
- [ ] Backup any local configuration

### Per-Crate Verification
- [ ] `logicaffeine_base` compiles standalone
- [ ] `logicaffeine_lexicon` compiles with build.rs lexicon generation
- [ ] `logicaffeine_kernel` has zero NL imports (grep for "parser", "lexer", "lexicon" - must find ZERO)
- [ ] `logicaffeine_data` compiles for wasm32-unknown-unknown (WASM-safe)
- [ ] `logicaffeine_system` compiles for both native and wasm32
- [ ] `logicaffeine_language` can parse without kernel dependency
- [ ] `logicaffeine_proof` certifies simple proofs
- [ ] `logicaffeine_compile` generates valid Rust code

### Integration Verification
- [ ] No circular dependencies between crates
- [ ] Feature flags work correctly (cli, verification)
- [ ] All 222 test files pass
- [ ] Phase 42 verification tests pass (with Z3)
- [ ] WASM build succeeds for web crates
- [ ] CI pipelines all green

### Infrastructure Verification
- [ ] Workers deploy successfully
- [ ] License validation works end-to-end
- [ ] Package registry accessible
- [ ] Frontend deploys to Cloudflare Pages

### Post-Migration
- [ ] Update README.md with new crate structure
- [ ] Update CLAUDE.md instructions
- [ ] Tag release: `git tag v0.6.0`
- [ ] Publish to crates.io (if ready)

---

## 9. Infrastructure & Deployment

### Cloudflare Workers (Remain Separate Node.js Projects)

| Worker | Purpose | Route | Storage |
|--------|---------|-------|---------|
| `worker/` | License validation via Stripe | `api.logicaffeine.com/*` | None |
| `registry/` | Package hosting & publishing | `registry.logicaffeine.com/*` | D1 + R2 |

#### License Validator (`worker/`)
- **File**: `worker/src/index.js`
- **Config**: `worker/wrangler.toml`
- **Routes**: `api.logicaffeine.com/*`
- **Secrets**: `STRIPE_SECRET_KEY`
- **Endpoints**:
  - `POST /validate` - Validate license key (sub_*, pi_*)
  - `POST /session` - Create session token
  - `GET /health` - Health check

#### Package Registry (`registry/`)
- **File**: `registry/src/index.js`
- **Config**: `registry/wrangler.toml`
- **Routes**: `registry.logicaffeine.com/*`
- **Database**: `logos-registry` (Cloudflare D1, ID: `fd52ddc6-fa8b-4e50-aed2-ee7d73437f61`)
- **Storage**: `logos-packages` (Cloudflare R2)
- **Secrets**: `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `JWT_SECRET`
- **Endpoints**:
  - `GET /packages` - List packages
  - `GET /packages/:name` - Package info
  - `GET /packages/:name/:version` - Version info
  - `GET /packages/:name/:version/download` - Download tarball
  - `POST /packages/publish` - Publish (auth required)

### CI/CD Workflows (`.github/workflows/`)

| Workflow | Trigger | Actions |
|----------|---------|---------|
| `test.yml` | Push/PR to main | `cargo build --verbose`, `cargo test` |
| `deploy-frontend.yml` | After tests pass on main | `dx build --release` → `wrangler pages deploy` |
| `deploy-registry.yml` | After tests pass on main | D1 migrations → `wrangler deploy` |

### Required GitHub Secrets
| Secret | Purpose |
|--------|---------|
| `CLOUDFLARE_API_TOKEN` | API access for Pages & Workers |
| `CLOUDFLARE_ACCOUNT_ID` | Account identifier |

### Worker Secrets (via `wrangler secret put`)
| Worker | Secrets |
|--------|---------|
| license-validator | `STRIPE_SECRET_KEY` |
| registry | `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `JWT_SECRET` |

---

## 10. Test Migration Strategy

### Test File Count: 222 files

### Distribution by Crate

| Crate | Test Files | Pattern |
|-------|------------|---------|
| `base` | 0 | Unit tests inline (`#[cfg(test)]`) |
| `lexicon` | 0 | Unit tests inline (`#[cfg(test)]`) |
| `kernel` | ~8 | `phase69_*.rs`, `phase70_*.rs`, `phase71_*.rs`, `phase72_*.rs`, `phase73_*.rs`, `phase99_*.rs` |
| `data` | ~15 | `phase_crdt_*.rs` (15 files) |
| `system` | ~10 | `phase49_*.rs`, `e2e_crdt*.rs`, `e2e_gossip*.rs` |
| `language` | ~55 | `phase1_*.rs` → `phase5_*.rs`, `phase7_*.rs`, `phase8_*.rs`, `phase10_*.rs`, `phase12_*.rs` → `phase20_*.rs`, `phase26_*.rs` |
| `proof` | ~20 | `phase35_*.rs`, `phase60_*.rs` → `phase68_*.rs`, `phase73_*.rs` → `phase78_*.rs` |
| `compile` | ~45 | `phase21_*.rs` → `phase25_*.rs`, `phase27_*.rs` → `phase38_*.rs`, `phase41_*.rs` → `phase48_*.rs`, `phase54_*.rs`, `phase55_*.rs`, `phase80_*.rs` → `phase103_*.rs` |
| `verify` | 1 | `phase_verification.rs` (requires `verification` feature) |
| `cli` | 0 | Integration tests at workspace level |
| `web` | 0 | Component tests (if any) in workspace |
| `workspace` | ~29 | `e2e_*.rs` (29 integration tests stay at root) |
| `misc` | ~15 | `debug_*.rs`, `*_tests.rs` (non-phase tests) |
| `utilities` | 2 | `common/mod.rs`, `extraction_common/mod.rs` |

### Migration Steps
1. **Create test crates**: `crates/<name>/tests/` for each crate
2. **Move phase tests**: Based on module dependencies
3. **Keep e2e_* at workspace**: These test cross-crate integration
4. **Update CI**: `cargo test --workspace` covers all

### Test Feature Flags
- Default: `cargo test --workspace -- --skip e2e`
- Full: `cargo test --workspace`
- Verification: `cargo test --workspace --features verification -- --skip e2e`

---

## 10.1 Feature Flag Test Matrix

The Cerf/Drasner feature flags in `logicaffeine_system` require comprehensive testing across all combinations.

### System Crate Feature Combinations

```bash
# Minimal (no features) - MUST work for simple script runners
cargo test -p logicaffeine_system -- --skip e2e

# Individual features
cargo test -p logicaffeine_system --features networking -- --skip e2e
cargo test -p logicaffeine_system --features persistence -- --skip e2e
cargo test -p logicaffeine_system --features concurrency -- --skip e2e

# Combined features
cargo test -p logicaffeine_system --features distributed -- --skip e2e
cargo test -p logicaffeine_system --features full -- --skip e2e
```

### CI Matrix Configuration

Add to `.github/workflows/test.yml`:

```yaml
jobs:
  test-system-features:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        features:
          - ""                    # Lean build
          - "networking"          # libp2p only
          - "persistence"         # Storage only
          - "concurrency"         # Rayon only
          - "distributed"         # networking + persistence
          - "full"                # Everything
    steps:
      - uses: actions/checkout@v4
      - name: Test system crate with features
        run: |
          cargo test -p logicaffeine_system --features "${{ matrix.features }}" -- --skip e2e
```

### Conditional Test Compilation

Tests that require specific features use `#[cfg(feature = "...")]`:

```rust
// tests/phase_system_features.rs

#[test]
fn io_always_available() {
    // This test runs regardless of features
    use logicaffeine_system::io;
    // io.rs is always available
}

#[test]
#[cfg(feature = "networking")]
fn gossip_requires_networking_feature() {
    use logicaffeine_system::network::gossip;
    // Only compiled when networking feature is enabled
}

#[test]
#[cfg(feature = "persistence")]
fn storage_requires_persistence_feature() {
    use logicaffeine_system::storage;
    // Only compiled when persistence feature is enabled
}

#[test]
#[cfg(feature = "concurrency")]
fn channels_require_concurrency_feature() {
    use logicaffeine_system::concurrency;
    // Only compiled when concurrency feature is enabled
}

#[test]
#[cfg(all(feature = "networking", feature = "persistence"))]
fn distributed_requires_both_features() {
    use logicaffeine_system::distributed::Distributed;
    // Only compiled when both features are enabled
}
```

### Feature Flag Coverage Requirements

| Feature | Required Tests | Modules Covered |
|---------|---------------|-----------------|
| (none) | `io_always_available` | `io.rs`, `time.rs`, `env.rs`, `random.rs` |
| `networking` | `gossip_*`, `mesh_*`, `protocol_*` | `network/*` |
| `persistence` | `storage_*`, `fs_*`, `file_*` | `storage/*`, `fs/*`, `file.rs` |
| `concurrency` | `channel_*`, `memory_*` | `concurrency.rs`, `memory.rs` |
| `distributed` | `distributed_*`, `e2e_crdt*` | `distributed.rs` + above |

### Local Feature Testing Script

Create `scripts/test-features.sh`:

```bash
#!/bin/bash
set -e

echo "Testing system crate feature combinations..."

FEATURES=("" "networking" "persistence" "concurrency" "distributed" "full")

for feature in "${FEATURES[@]}"; do
    echo "=== Testing with features: '${feature:-none}' ==="
    if [ -z "$feature" ]; then
        cargo test -p logicaffeine_system -- --skip e2e
    else
        cargo test -p logicaffeine_system --features "$feature" -- --skip e2e
    fi
done

echo "All feature combinations passed!"
```

---

## 11. Assets & Build System

### Build Script Migration

The current `build.rs` (1664 lines, 64KB) performs:
1. Reads `assets/lexicon.json` (185KB vocabulary database)
2. Generates `lexicon_data.rs` with 40+ lookup functions
3. Generates `mwe_data.rs` for multi-word expressions
4. Generates `ontology_data.rs` for part-whole relations
5. Generates `axiom_data.rs` for meaning postulates

**Migration Plan:**
- `build.rs` moves to `crates/logicaffeine_lexicon/build.rs`
- Output files go to `OUT_DIR` as today
- Language crate depends on `lexicon` which includes the generated code

### Asset Distribution

| Asset | Current Location | Target |
|-------|-----------------|--------|
| `lexicon.json` | `assets/` | `lexicon/assets/` - processed by build.rs |
| `curriculum/` | `assets/curriculum/` | `web/assets/` - embedded via `include_dir` |
| `std/*.lg` | `assets/std/` | `compile/assets/` - standard library |
| `logo.svg` | `assets/` | `web/assets/` |
| `logo.jpeg` | `assets/` | `web/assets/` |
| `favicon.svg` | `assets/` | `web/assets/` |
| `og-image.svg` | `assets/` | `web/assets/` |

### Documentation Scripts
These need path updates post-migration:
| Script | Purpose |
|--------|---------|
| `generate-docs.sh` | Master docs generator (17KB) |
| `generate-frontend-docs.sh` | UI component docs |
| `generate-imperative-docs.sh` | LOGOS language docs |
| `generate-logical-docs.sh` | FOL semantics docs |

---

## 12. Risk Mitigation

### Potential Breaking Points

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Circular dependencies | Medium | Build fails | Run `cargo tree -p <crate>` after each extraction |
| Missing re-exports | High | Compile errors | Grep for `use logos::` and map to new crate paths |
| Build.rs lexicon failure | Medium | Lexicon broken | Test `cargo build -p logicaffeine_lexicon` first |
| Build.rs complexity (1663 lines) | Medium | Lexicon broken | Verify all 4 generated files exist with expected sizes |
| WASM target breaks | Medium | Web broken | Test `--target wasm32-unknown-unknown` per crate |
| CI pipeline fails | Medium | No deployments | Update workflows incrementally, test each |
| Type conflicts | Low | Compile errors | Ensure base types in base crate, re-export correctly |
| Dynamic lexicon API drift | Low | Runtime errors | Keep `runtime.rs` API in sync with `build.rs` output |
| Missing workspace exclusion | Medium | Build fails | Verify `exclude = ["crates/logicaffeine_verify"]` in root Cargo.toml |

### Build Script Verification

After lexicon extraction, verify generated files:
```bash
# Check all generated files exist and have expected sizes
find target -name "lexicon_data.rs" -exec wc -l {} \;   # Should be 10,000+ lines
find target -name "mwe_data.rs" -exec wc -l {} \;       # Should exist
find target -name "ontology_data.rs" -exec wc -l {} \;  # Should exist
find target -name "axiom_data.rs" -exec wc -l {} \;     # Should exist
```

### Rollback Strategy
```bash
# Before starting
git tag pre-packaging
git checkout -b feature/tiered-crates

# If blocked at any phase
git stash
git checkout main
# Reassess and try different approach

# If completed but broken
git checkout pre-packaging
```

### Incremental Verification Checklist
After EACH phase:
- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace -- --skip e2e`
- [ ] `cargo build -p logicaffeine_cli --features cli` (if CLI affected)
- [ ] `dx build --release` (WASM check)
- [ ] No new warnings in CI

### Dependency Audit Commands
```bash
# Check for cycles
cargo tree --workspace -e normal --prefix depth

# Check feature propagation
cargo tree -p logicaffeine_cli --features cli

# Check WASM compatibility
cargo build -p logicaffeine_base --target wasm32-unknown-unknown
cargo build -p logicaffeine_data --target wasm32-unknown-unknown
cargo build -p logicaffeine_language --target wasm32-unknown-unknown
```

---

## 12.1 Regression Prevention Protocol

**From CLAUDE.md**: "If a test is failing it is ALWAYS A REGRESSION. We do not move forward until ALL TESTS PASS."

This section codifies the Iron Rule into actionable protocol.

### The Iron Rule

```
NEVER modify a failing test to make it pass.
The test is the specification. Fix the implementation.
```

### Pre-Commit Verification

Run before EVERY commit during the refactor:

```bash
#!/bin/bash
# .git/hooks/pre-commit (optional but recommended)
set -e

echo "Running pre-commit verification..."
cargo test -- --skip e2e

if [ $? -ne 0 ]; then
    echo "❌ BLOCKED: Tests are failing. Fix before committing."
    echo "   DO NOT modify the test. Fix the implementation."
    exit 1
fi

echo "✓ All tests pass. Commit allowed."
```

### Checkpoint Commands

| Context | Command | When to Use |
|---------|---------|-------------|
| Quick check | `cargo test -- --skip e2e` | After each file move |
| Full check | `cargo test` | Before PR, after each phase |
| Feature check | `cargo test --features verification` | After verify crate changes |
| WASM check | `cargo build --target wasm32-unknown-unknown` | After data/language changes |

### Test Count Tracking Log

Maintain this log during the refactor. Test count should NEVER decrease.

```markdown
# Test Count Log

| Date | Phase | Test Files | Tests Run | Tests Passed | Status |
|------|-------|------------|-----------|--------------|--------|
| START | Pre-refactor | 222 | ~2,536 | ~2,536 | ✓ Baseline |
| | Phase 1.1 (base) | 222 | ~2,536 | | |
| | Phase 1.2 (lexicon) | 222 | ~2,536 | | |
| | Phase 1.3 (kernel) | 222 | ~2,536 | | |
| | Phase 2.1 (data) | 222 | ~2,536 | | |
| | Phase 3.1 (system) | 222 | ~2,536 | | |
| | Phase 3.2 (language) | 222 | ~2,536 | | |
| | Phase 3.3 (proof) | 222 | ~2,536 | | |
| | Phase 3.4 (compile) | 222 | ~2,536 | | |
| | Phase 3.5 (verify) | 222 | ~2,536 | | |
| | Phase 4.1 (apps) | 222 | ~2,536 | | |
| | Phase 4.2 (CI) | 222 | ~2,536 | | |
| END | Final | 222+ | ~2,536+ | ~2,536+ | ✓ Complete |
```

### Recovery Protocol

When a test fails during refactoring:

```
┌─────────────────────────────────────────────────────────────┐
│                    TEST FAILURE PROTOCOL                     │
├─────────────────────────────────────────────────────────────┤
│ 1. STOP immediately                                         │
│    - Do not make more changes                               │
│    - Do not "try one more thing"                            │
│                                                             │
│ 2. DO NOT touch the test file                               │
│    - The test is correct                                    │
│    - The implementation is wrong                            │
│                                                             │
│ 3. Identify the failure                                     │
│    - What changed?                                          │
│    - What import broke?                                     │
│    - What re-export is missing?                             │
│                                                             │
│ 4. Options:                                                 │
│    a) Fix forward: Add missing re-export/import             │
│    b) Rollback: git stash && retry with smaller change      │
│    c) Ask: If unclear, ask user before proceeding           │
│                                                             │
│ 5. Verify fix                                               │
│    - cargo test -- --skip e2e                               │
│    - Must show: 0 failures                                  │
│                                                             │
│ 6. Continue only when green                                 │
└─────────────────────────────────────────────────────────────┘
```

### Common Failure Patterns and Fixes

| Failure Pattern | Root Cause | Fix |
|-----------------|------------|-----|
| `cannot find X in crate` | Missing re-export | Add `pub use` in new crate's `lib.rs` |
| `private type in public interface` | Visibility issue | Make type `pub` or re-export from correct module |
| `circular dependency` | Crate A imports B, B imports A | Refactor shared types to `base` crate |
| `unresolved import` | Old path used | Update `use` statement to new crate path |
| `trait bound not satisfied` | Missing impl in new location | Move trait impl with type, or add feature gate |

### Architectural Invariant Tests

These tests verify the Council's mandates are maintained:

```rust
// tests/architectural_invariants.rs

/// Milner: Kernel has no path to lexicon
#[test]
fn milner_invariant() {
    let output = std::process::Command::new("cargo")
        .args(["tree", "-p", "logicaffeine_kernel"])
        .output()
        .expect("cargo tree failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("lexicon"), "MILNER VIOLATION: kernel depends on lexicon");
}

/// Liskov: Proof has no path to language
#[test]
fn liskov_invariant() {
    let output = std::process::Command::new("cargo")
        .args(["tree", "-p", "logicaffeine_proof"])
        .output()
        .expect("cargo tree failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("language"), "LISKOV VIOLATION: proof depends on language");
}

/// Lamport: Data is WASM-safe
#[test]
fn lamport_invariant() {
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "logicaffeine_data", "--target", "wasm32-unknown-unknown"])
        .status()
        .expect("cargo build failed");
    assert!(status.success(), "LAMPORT VIOLATION: data crate not WASM-safe");
}
```

---

## 13. Versioning Strategy

### Initial Versions Post-Migration

| Crate | Version | Stability |
|-------|---------|-----------|
| `logicaffeine` (root) | 0.6.0 | Breaking (new structure) |
| `logicaffeine_base` | 0.1.0 | Stable API |
| `logicaffeine_lexicon` | 0.1.0 | Stable API |
| `logicaffeine_kernel` | 0.1.0 | Stable API |
| `logicaffeine_data` | 0.1.0 | Stable API |
| `logicaffeine_system` | 0.1.0 | Evolving |
| `logicaffeine_language` | 0.1.0 | Semi-stable |
| `logicaffeine_proof` | 0.1.0 | Evolving |
| `logicaffeine_compile` | 0.1.0 | Evolving |
| `logicaffeine_verify` | 0.1.0 | Experimental |
| `logicaffeine_cli` | 0.6.0 | Matches root |
| `logicaffeine_web` | 0.6.0 | Matches root |

### Semantic Versioning Policy

| Change Type | Version Bump |
|-------------|--------------|
| Bug fix | Patch (0.1.X) |
| New feature (backward compatible) | Minor (0.X.0) |
| Breaking API change | Major (X.0.0) |

### Inter-Crate Dependencies
```toml
# Initial: Exact versions for safety
logicaffeine-base = "=0.1.0"

# After stabilization: Compatible versions
logicaffeine-base = "^0.1"
```

### Changelog
Each crate needs `CHANGELOG.md` tracking:
- Version number
- Release date
- Breaking changes (if any)
- New features
- Bug fixes

---

## Appendix: Rationale for Decomposition

### Why Not a Single `logicaffeine_core`?

Module coupling analysis revealed distinct boundaries:

| Module Group | Coupling | Recommendation |
|--------------|----------|----------------|
| Base atoms (6 files) | Zero deps | Separate crate ✓ |
| Lexicon (generated) | English only | Separate crate ✓ |
| Kernel (14 files) | Isolated, zero NL deps | Separate crate ✓ |
| Data (CRDTs, types) | WASM-safe, no IO | Separate crate ✓ |
| System (IO, network) | Platform-specific | Separate crate ✓ |
| Parser/Lexer/AST | Tightly coupled | Keep together in `language` |
| Proof engine | Moderate coupling | Separate, depends on kernel |
| Analysis/Codegen | Tightly coupled | Keep together in `compile` |

### Benefits of Tiered Architecture

1. **Faster compiles** for users who only need parsing (~315KB vs ~800KB)
2. **Standalone kernel** valuable for type theory enthusiasts
3. **WASM-lean data** - CRDTs without libp2p in browser
4. **Clean API boundaries** at natural coupling points
5. **Independent versioning** for stable components (kernel, base, data)
6. **Easier testing** of isolated components
