# Friend Plans

Notes from a deep conversation about what LogicAffeine could become. Each section covers an idea that came up, how it connects to what we already have, what it would unlock, what it takes to build, and where the integration points are.

---

## Table of Contents

1. [Escape Hatches](#1-escape-hatches)
2. [FFI — Foreign Function Interface](#2-ffi--foreign-function-interface)
3. [Super-Compilation & Partial Evaluation](#3-super-compilation--partial-evaluation)
4. [Stable Portable ABI (Cap'n Proto)](#4-stable-portable-abi-capn-proto)
5. [Structural Subtyping](#5-structural-subtyping)
6. [Cross-Compilation & WASM](#6-cross-compilation--wasm)
7. [Fast Compilation](#7-fast-compilation)
8. [Memory Management & GC](#8-memory-management--gc)
9. [Macros & Homoiconicity](#9-macros--homoiconicity)
10. [Jepsen — Distributed Correctness Testing](#10-jepsen--distributed-correctness-testing)
11. [Susan Haack — Philosophy of Logics](#11-susan-haack--philosophy-of-logics)
12. [Godbolt — Compiler Explorer](#12-godbolt--compiler-explorer)
13. [Fully Homomorphic Neural Networks](#13-fully-homomorphic-neural-networks)
14. [Unison — Content-Addressed Code](#14-unison--content-addressed-code)
15. [Inigo Quilez — Shader Math & Creative Coding](#15-inigo-quilez--shader-math--creative-coding)
16. [Laziness](#16-laziness)
17. [HVM2 & Interaction Nets](#17-hvm2--interaction-nets)
18. [Esperanto — Universal Language](#18-esperanto--universal-language)
19. [The Glue Language Vision](#19-the-glue-language-vision)
20. [Eliminating the Compile/Interpret/Run Distinction](#20-eliminating-the-compileinterpretrun-distinction)

---

## 1. Escape Hatches

**Priority: HIGHEST. This is the single most important feature for adoption.**

### The Idea

When the language can't express something, there has to be a clean way to drop down to the host language without leaving the file, without breaking the abstraction, without fighting the tools. Every new language dies when a user hits a wall and has no way through it. Escape hatches turn walls into doors.

### What We Have Today

Nothing. If LOGOS can't express something, you can't write it in LOGOS. The generated Rust is a build artifact that users aren't supposed to touch.

### What It Unlocks

- Users can adopt LOGOS incrementally — write what they can in English, escape to Rust for the rest
- No feature request is ever truly blocking — the escape hatch is always available
- Library authors can wrap high-performance Rust code in LOGOS interfaces
- Interop with any Rust crate becomes possible immediately without waiting for native LOGOS support
- Enables every other interop feature in this document

### What It Takes to Build

**AST**: New statement variant in `crates/logicaffeine_language/src/ast/stmt.rs`:

```rust
Escape {
    language: Symbol,    // "Rust" for now, could support others later
    bindings_in: Vec<Symbol>,   // LOGOS variables available inside the block
    bindings_out: Option<Symbol>, // optional return value
    code: String,        // raw foreign code
    span: Span,
}
```

**Parser** (`crates/logicaffeine_language/src/parser/mod.rs`): After `Escape to Rust:`, consume everything at the next indentation level as a raw string. No parsing of the content — it's opaque foreign code. The parser needs to track which LOGOS variables are in scope so the codegen can bind them.

**CodeGen** (`crates/logicaffeine_compile/src/codegen.rs`): Emit the raw string directly into the generated Rust function. Wrap it in a block `{ ... }` with variable bindings from the enclosing scope. The last expression in the block becomes the return value if the escape is used in expression position.

**Analysis**: Escape blocks are opaque to ownership and escape analysis. Conservative approach: any variable referenced in the escape block is considered consumed (moved). If the user wants to keep using it after the escape, they need to `Copy` it first. This is safe — it might reject valid programs, but it never accepts invalid ones.

### Integration Points

- `ast/stmt.rs` — new variant
- `parser/mod.rs` — new parse rule, raw string consumption
- `codegen.rs` — passthrough emission with binding wrappers
- `analysis/ownership.rs` — conservative handling (assume consumption)
- `analysis/escape.rs` — zone analysis (escape blocks can't reference zone-local values)
- `interpreter.rs` — can't interpret escape blocks; error with "this program requires compilation"

### Language Surface

```logos
## To compress (data: Seq of Int) -> Seq of Int:
    Let header be [1, 0, 0].

    Escape to Rust:
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&data.as_bytes()).unwrap();
        encoder.finish().unwrap()

    Push header to result.
    Return result.
```

The `data` variable from LOGOS scope is available inside the escape block as its generated Rust type. The last expression is the block's value.

---

## 2. FFI — Foreign Function Interface

### The Idea

Not just escaping to raw code, but structured foreign function bindings. Inline FFI (embed foreign code) vs. Template FFI (declare foreign signatures). Bidirectional — LOGOS calls foreign code AND foreign code calls LOGOS. Lightweight, not ceremonial.

### What We Have Today

`FunctionDef` already has an `is_native` flag in the AST. It's wired up but doesn't do anything in codegen yet. The compiler outputs Rust, so Rust's FFI machinery (`extern "C"`, `#[no_mangle]`, `#[wasm_bindgen]`) is available in generated code.

### What It Unlocks

- **Inbound FFI**: Call any Rust crate function from LOGOS
- **Outbound FFI**: Export LOGOS functions so C, Python, JS, etc. can call them
- **Dependency management**: LOGOS projects can declare Rust crate dependencies
- **Ecosystem access**: The entire crates.io ecosystem becomes available to LOGOS programs
- **Embedding**: LOGOS as a scripting language inside Rust applications

### What It Takes to Build

**Four tiers, increasing in complexity:**

**Tier 1: Dependency Declarations**

```logos
## This project uses:
    The "serde" crate version "1.0" for serialization.
    The "reqwest" crate version "0.11" with features "json" and "blocking" for HTTP.
```

Parser recognizes this as a dependency block. `largo build` writes the dependencies into the generated `Cargo.toml`. This is the easiest tier — mostly string manipulation in the build system.

Integration: `apps/logicaffeine_cli/src/project/build.rs` — inject dependencies into generated Cargo.toml. `ast/stmt.rs` — new `Dependency` variant or special metadata block.

**Tier 2: Native Function Declarations**

```logos
## To fetch (url: Text) -> Text is native "reqwest::blocking::get".
## To parse_json (text: Text) -> Map of Text to Text is native "serde_json::from_str".
```

The `is_native` flag already exists. Codegen needs to emit a wrapper function that calls the named Rust path, converting between LOGOS types and Rust types.

Integration: `codegen.rs` — handle `is_native` in function codegen. Need a type mapping table: LOGOS `Text` → Rust `String`, LOGOS `Int` → Rust `i64`, LOGOS `Seq of T` → Rust `Vec<T>`, etc. This table mostly exists already since codegen translates types.

**Tier 3: Bidirectional / Exported Functions**

```logos
## To handle_request (req: Request) -> Response is exported:
    Let name be req's name.
    Return a new Response with body "Hello " ++ name.
```

Codegen emits `#[no_mangle] pub extern "C" fn handle_request(...)` for native targets, or `#[wasm_bindgen] pub fn handle_request(...)` for WASM targets. Requires generating C-compatible type wrappers for portable structs.

Integration: `codegen.rs` — new attribute emission. `ast/stmt.rs` — `is_exported` flag on FunctionDef. Type marshaling layer for C ABI types.

**Tier 4: Inline Foreign Blocks (subsumes Escape Hatches)**

This is Escape Hatches (section 1) generalized. The escape hatch IS the inline FFI.

### Integration Points

- `ast/stmt.rs` — `is_native`, `is_exported` flags, `Dependency` blocks
- `parser/mod.rs` — parse "is native", "is exported", dependency blocks
- `codegen.rs` — native function wrappers, export attributes, type marshaling
- `apps/logicaffeine_cli/src/project/build.rs` — Cargo.toml dependency injection
- `apps/logicaffeine_cli/src/project/manifest.rs` — dependency tracking in Largo.toml

---

## 3. Super-Compilation & Partial Evaluation

### The Idea

The compiler should do as much work as possible at compile time. If a function gets called with known arguments, evaluate it during compilation. Don't distinguish between "compile time" and "run time" — the compiler is an evaluator that stops when it runs out of information. Currying a function with the first argument? Evaluate everything that depends only on that argument right now. The rest waits for runtime.

This is related to Futamura projections, partial evaluation, and supercompilation. The HVM2/interaction net approach (section 17) is one way to implement it. The key insight: compilation and interpretation merge. Your compiler IS an interpreter that also emits code.

### What We Have Today

Zero optimization passes. No constant folding, no dead code elimination, no compile-time evaluation. The pipeline is linear: parse → analyze → emit Rust → let LLVM optimize.

But here's the thing — we have BOTH a compiler AND an interpreter in `logicaffeine_compile`. The interpreter (`interpreter.rs`) can already evaluate LOGOS programs. The compiler (`codegen.rs`) emits Rust. These two live in the same crate. That's exactly the architecture partial evaluation needs.

### What It Unlocks

- Dramatically better generated code — constants folded, dead branches eliminated, specialized functions
- "Zero-cost abstractions" — generic functions specialized for their actual uses
- Blurred compile/runtime boundary that makes the language feel like a REPL even when compiled
- Functions that are expensive at runtime but cheap when arguments are known become free
- Natural path to JIT-like behavior without a JIT
- Programs get faster without any user effort

### What It Takes to Build

**Stage 1: Constant Folding (weeks, not months)**

After parsing, walk the AST. Any `BinaryOp` where both sides are `Literal` → evaluate and replace with the result.

```logos
Let tau be 2 * 3.14159.        ## Folds to: Let tau be 6.28318.
Let big be 1000 * 1000 * 1000. ## Folds to: Let big be 1000000000.
```

Integration: New pass between parsing and codegen. `crates/logicaffeine_compile/src/optimize/fold.rs` (new file). Walk `Vec<Stmt>`, pattern-match on `Expr::BinaryOp` with literal operands.

**Stage 2: Dead Code Elimination (weeks)**

After constant folding, if-branches with literal conditions can be pruned.

```logos
Let debug be False.
If debug:             ## Entire branch eliminated
    Print "debug info".
```

Integration: Same optimization pass. Pattern-match on `Stmt::If` with `Expr::Literal(Bool(false))` condition → remove the if, keep the else (or vice versa).

**Stage 3: Function Specialization (months)**

When a function is called with some known arguments, generate a specialized version.

```logos
## To multiply (a: Int) and (b: Int) -> Int:
    Return a * b.

Let double be multiply with a 2.   ## Generates: fn double(b: i64) -> i64 { 2 * b }
```

This requires partial application support in the language (which connects to laziness, section 16). The compiler detects known arguments and generates specialized function variants.

Integration: `codegen.rs` — when emitting a function call where some arguments are constants, emit a specialized inline version. Or maintain a specialization cache that generates new function definitions.

**Stage 4: Compile-Time Evaluation (months to years)**

Use the existing interpreter to evaluate pure functions at compile time.

```logos
## To factorial (n: Int) -> Int:
    If n < 2: Return 1.
    Return n * factorial(n - 1).

Let fact_10 be factorial(10).  ## Compiler evaluates to 3628800, emits: let fact_10 = 3628800;
```

The compiler needs to determine which functions are "pure" (no side effects, no IO). For pure functions with all-constant arguments, run the interpreter and replace the call with its result.

Integration: `interpreter.rs` — add a `evaluate_pure(expr, env) -> Option<Value>` mode. `codegen.rs` — before emitting a function call, try compile-time evaluation. Fall back to normal codegen if evaluation fails (unknown arguments, side effects, timeout).

**Stage 5: Supercompilation Proper (research-grade)**

The evaluator doesn't just handle all-or-nothing — it partially evaluates. Given `multiply(2, b)` where `b` is unknown, it produces `2 * b` without the function call overhead. Given `if (true && x)` it produces `if x`. Every expression is simplified as far as current knowledge allows.

This is Futamura's second projection: a partial evaluator that, given a program and partial input, produces a specialized residual program.

Integration: This would be a new major component — `crates/logicaffeine_compile/src/optimize/partial_eval.rs` — that combines the interpreter's evaluation capability with the codegen's code emission. It walks the AST, evaluating what it can and emitting code for the rest.

### Where the Integration Points Are

- New directory: `crates/logicaffeine_compile/src/optimize/`
  - `fold.rs` — constant folding
  - `dce.rs` — dead code elimination
  - `specialize.rs` — function specialization
  - `partial_eval.rs` — the big one
- `compile.rs` — insert optimization passes between analysis and codegen
- `interpreter.rs` — extract pure evaluation into a reusable component
- `codegen.rs` — accept pre-optimized AST

---

## 4. Stable Portable ABI (Cap'n Proto)

### The Idea

A binary layout for data that doesn't change between compiler versions, languages, or platforms. Zero-copy serialization — the wire format IS the in-memory format. No encode/decode step. Cap'n Proto is the gold standard for this.

### What We Have Today

`StructDef` already has `is_portable: bool`. This flag exists in the AST, gets parsed, and survives through to codegen. Right now it doesn't change the generated code, but the intent is already declared.

CRDTs need serialization for journal persistence (`Mount`) and network transmission (`Sync`, `SendMessage`). The current wire format for these isn't documented or stable.

### What It Unlocks

- **FFI boundary type**: Portable structs are the natural type for crossing language boundaries. C, Python, JS, Rust all read the same bytes.
- **Network wire format**: Agents sending messages use portable structs. Zero-copy deserialization means no parsing overhead for high-throughput messaging.
- **Persistent storage**: Journal files use the same format. An old journal written by v1.0 is readable by v2.0.
- **Schema evolution**: Cap'n Proto supports adding fields, deprecating fields, and maintaining backwards compatibility.
- **WASM interop**: Portable structs can cross the WASM boundary without serialization — shared memory with a stable layout.

### What It Takes to Build

**Phase 1: Schema Generation**

When `is_portable` is true, generate a Cap'n Proto schema alongside the Rust struct:

```capnp
struct Point @0xa1b2c3d4e5f6a7b8 {
    x @0 :Int64;
    y @1 :Int64;
}
```

Integration: `codegen.rs` — when emitting a struct with `is_portable`, also emit a `.capnp` schema file. Add `capnp` and `capnpc-rust` as build dependencies. Generate both the Rust struct (for normal use) and the Cap'n Proto bindings (for FFI/network/storage).

**Phase 2: Transparent Conversion**

LOGOS code uses portable structs normally. Codegen handles conversion at boundaries:

```logos
## A Message has (portable):
    A sender: Text.
    A body: Text.
    A timestamp: Int.

Send message to agent.  ## Serializes via Cap'n Proto at the boundary
```

The user never writes serialization code. Codegen emits `.to_capnp()` at send boundaries and `.from_capnp()` at receive boundaries.

**Phase 3: Zero-Copy Access**

For read-heavy workloads, skip deserialization entirely. Cap'n Proto readers access fields directly from the byte buffer.

```logos
Receive msg from pipe.
Print msg's body.       ## No deserialization — reads directly from buffer
```

Integration: Type-directed codegen. When the next operation after receiving is read-only field access, emit Cap'n Proto reader code instead of full deserialization.

### Integration Points

- `codegen.rs` — portable struct code generation, boundary serialization
- `ast/stmt.rs` — `is_portable` already exists, may need schema version metadata
- `apps/logicaffeine_cli/src/project/build.rs` — Cap'n Proto build step
- `crates/logicaffeine_system/` — replace ad-hoc wire formats in networking/journal with Cap'n Proto
- `crates/logicaffeine_data/` — CRDT state serialization via Cap'n Proto (Lamport-safe since capnp is pure)

---

## 5. Structural Subtyping

### The Idea

"Don't want to jump through 7 hoops to say this type is equal to that type." If two types have the same shape, they should be compatible. A `Point { x: Int, y: Int }` and a `Coordinate { x: Int, y: Int }` should be interchangeable where only the shape matters.

### What We Have Today

Types are nominal. A `Point` is a `Point` and a `Coordinate` is a `Coordinate`, even if they have identical fields. The type system has `TypeExpr::Named(Symbol)` which carries only the name, not the structure.

Refinement types (`Int where it > 0`) add predicate-based subtyping, but only for primitive types.

### What It Unlocks

- Functions that accept "anything with an x and y field" without forcing a shared base type
- Natural English phrasing: "any thing with a name and an age" reads better than "any NamedAged"
- Protocol compliance: if your type has all the right fields, it implements the protocol. No explicit `impl`
- Gradual typing: start with structural types, name them later if you want
- Better interop: foreign types that happen to have the right fields just work

### What It Takes to Build

**Approach: Row Polymorphism**

Instead of "any type", allow type constraints that specify required fields:

```logos
## To print_name (thing: any with name: Text):
    Print thing's name.
```

The TypeExpr AST needs a new variant:

```rust
TypeExpr::Structural {
    required_fields: Vec<(Symbol, TypeExpr)>,
}
```

**Codegen strategy**: Generate a Rust trait with getter methods for each required field. Auto-implement the trait for any struct that has those fields:

```rust
trait HasName {
    fn name(&self) -> &str;
}

impl HasName for Person { fn name(&self) -> &str { &self.name } }
impl HasName for Company { fn name(&self) -> &str { &self.name } }

fn print_name(thing: &impl HasName) {
    println!("{}", thing.name());
}
```

The trait and implementations are generated automatically by codegen based on the structural type constraint and the struct definitions in the TypeRegistry.

### Integration Points

- `ast/stmt.rs` or `ast/mod.rs` — new `TypeExpr::Structural` variant
- `crates/logicaffeine_language/src/analysis/discovery.rs` — register structural constraints during discovery
- `codegen.rs` — generate traits for structural types, auto-impl for matching structs
- `parser/mod.rs` — parse "any with field: Type and field: Type" syntax
- `crates/logicaffeine_compile/src/analysis/` — type checking for structural compatibility

---

## 6. Cross-Compilation & WASM

### The Idea

Compile on one machine, target every platform. WASM as the universal binary format. Don't need a CI farm with different architectures — one big build server, all targets.

### What We Have Today

Extensive WASM support already exists:
- `wasm32-unknown-unknown` documented as a target in workspace Cargo.toml
- `logicaffeine_data` uses `getrandom` with `js` feature for WASM
- `logicaffeine_system` has full WASM conditional compilation — VFS over FileSystem API, IndexedDB fallback, `web-sys` DOM access
- `logicaffeine_compile` uses `gloo-timers` for WASM-compatible async sleep
- `logicaffeine_web` is a full Dioxus web IDE compiled to WASM

What's missing is a user-facing `--target` flag on `largo build`.

### What It Unlocks

- `largo build --target wasm` — one command to produce a universal binary
- WASM programs run in browsers, on servers (wasmtime/wasmer), and embedded (WASI)
- CI pipelines simplify from 6 architecture-specific builds to 1 WASM build + optional native builds
- LOGOS-as-plugin: WASM modules can be loaded by any host language
- Sandboxed execution: WASM's capability model aligns with LOGOS's `Check` security primitives

### What It Takes to Build

**Phase 1: `--target` flag** (straightforward)

```
largo build --target wasm     # wasm32-wasi
largo build --target native   # default, current behavior
largo build --target web      # wasm32-unknown-unknown + wasm-bindgen
```

Integration: `apps/logicaffeine_cli/src/cli.rs` — add `--target` to BuildConfig. `apps/logicaffeine_cli/src/project/build.rs` — pass `--target wasm32-wasi` or `--target wasm32-unknown-unknown` to cargo build.

**Phase 2: Feature-gating by target**

Some LOGOS features (networking, file IO, memory-mapped zones) aren't available on WASM. The compiler should warn at parse-time, not at Rust-compile-time:

```
Warning: 'Listen on address' is not available on WASM target.
This program will fail to compile for --target wasm.
```

Integration: `codegen.rs` — when target is WASM, check each statement for WASM-incompatible features. Emit warnings or errors with LOGOS-level source locations.

**Phase 3: WASI preview 2**

WASI preview 2 adds sockets, which would make networking available on WASM. The system crate's networking code would need a WASI backend alongside the libp2p backend.

---

## 7. Fast Compilation

### The Idea

Extremely fast compile-and-run cycle. No waiting seconds or minutes to see if your change works.

### What We Have Today

Two-stage compilation:
1. LOGOS → Rust (our compiler, fast — arenas, interning, single-pass)
2. Rust → binary (rustc + LLVM, slow — the bottleneck, 2-30+ seconds)

The interpreter (`interpreter.rs`) in `logicaffeine_compile` can evaluate programs without the Rust compilation step. The `interpreter-only` feature flag exists but isn't exposed in the CLI.

### What It Unlocks

- Sub-second feedback loop for development
- REPL-like experience for a compiled language
- Faster CI pipelines
- Lower barrier to experimentation

### What It Takes to Build

**Tier 1: `largo run --interpret` (easy)**

Add `--interpret` flag that uses the existing interpreter instead of going through Rust compilation. This should be the default for development.

Integration: `apps/logicaffeine_cli/src/cli.rs` — add flag. Call `interpret_program(source)` instead of `compile_and_run(source)`. The interpreter already exists in `crates/logicaffeine_compile/src/interpreter.rs`.

**Tier 2: Incremental codegen (medium)**

Only regenerate Rust for functions that changed. Cache `function-source-hash → generated-rust` mappings.

Integration: New caching layer in `crates/logicaffeine_compile/src/` that hashes function bodies and only reruns codegen for changed functions. The generated Rust project keeps unchanged function files.

**Tier 3: Cranelift backend (hard)**

Instead of LLVM, use Cranelift for debug builds. Much faster compilation, slower generated code. This is what rustc's `-Z codegen-backend=cranelift` does.

Integration: `apps/logicaffeine_cli/src/project/build.rs` — detect if cranelift backend is available, use it for non-release builds.

**Tier 4: Direct machine code emission (research)**

Skip Rust entirely. Emit machine code directly from LOGOS AST using Cranelift as a library. No intermediate Rust source, no cargo build. This would give single-digit-millisecond compile times.

Integration: Entirely new backend in `crates/logicaffeine_compile/src/`. Would live alongside `codegen.rs` as an alternative code generation target.

---

## 8. Memory Management & GC

### The Idea

"Don't want to fuck around with my own memory 99% of the time." Garbage collection. The user writes logic, the runtime handles allocation and deallocation.

### What We Have Today

- **Ownership semantics**: `Give` (move) and `Show` (borrow) provide explicit control
- **Zone-based arenas**: `Inside a new zone called 'Scratch':` scopes memory
- **Liberal cloning in codegen**: For-in loops clone collections, `Copy` clones values. This hides memory complexity.
- **Rust's Drop**: Generated Rust uses RAII — heap memory freed when variables go out of scope

The current experience is mostly GC-like already. Users don't write `free()` or `dealloc()`. They might hit use-after-move errors, but the ownership checker catches those at compile time with helpful messages.

### What It Unlocks (with improvement)

- Zero move errors for casual users — everything "just works"
- Circular data structures without fighting the borrow checker
- Higher-level programming model (closer to Python/JS ease of use)
- Simpler generated code (no ownership gymnastics)

### What It Takes to Build

**Option A: Rc<T> by Default (pragmatic, recommended)**

Generate `Rc<T>` (or `Arc<T>` for concurrent code) for heap-allocated types. Reference counting IS garbage collection — deterministic, low-overhead, no pauses.

```rust
// Current codegen:
let point = Point { x: 10, y: 20 };

// Rc codegen:
let point = Rc::new(Point { x: 10, y: 20 });
```

Cloning an `Rc` is cheap (increment a counter). Move errors go away because everything is shared. The `Give` keyword becomes `Rc::clone()` — ownership transfer is just sharing the reference.

Cycle detection: Add `weak` references for parent pointers, or use a cycle-collecting library. In practice, LOGOS programs rarely create cycles because the language doesn't have raw pointers.

Integration: `codegen.rs` — wrap struct types in `Rc<>`, convert field access to `Rc::clone()` + deref. `analysis/ownership.rs` — relax move checking for Rc-wrapped types.

**Option B: Keep Current + Better Errors**

The current approach is fine for most programs. Improve the error messages when users hit move errors, and make `Copy` more automatic:

```logos
Give x to y.
Print x.        ## Error: x was given away. Did you mean to use Copy?
                ## Suggestion: Let y be a Copy of x. (keeps original)
```

Integration: `analysis/ownership.rs` — better diagnostics. No codegen changes.

**Option C: Target-Dependent**

- `largo build --target native` → Rust ownership model (fast, no GC)
- `largo build --target wasm` → Rc-based (easier, GC-compatible)
- `largo run --interpret` → interpreter uses its own Value type (already GC'd by Rust)

### Integration Points

- `codegen.rs` — type wrapping strategy
- `analysis/ownership.rs` — relaxed checking for RC types
- `crates/logicaffeine_compile/src/types.rs` — type representation decisions

---

## 9. Macros & Homoiconicity

### The Idea

Super easy Lisp/Clojure-style macros. Code is data, data is code. Programs can inspect and transform their own structure. The homoiconic representation where the syntax tree is a first-class value in the language.

### What We Have Today

No macro system. No metaprogramming. The AST exists in Rust but is inaccessible from LOGOS programs. There are template patterns (EventTemplate, noun phrase composition) but these are parser internals, not user-facing.

### What It Unlocks

- Domain-specific languages (DSLs) inside LOGOS
- Code generation and transformation without external tools
- Decorator/annotation patterns (logging, timing, validation)
- User-defined control flow (retry, circuit-breaker, etc.)
- Library-level language extensions

### What It Takes to Build

**Approach A: Template Macros (6 weeks)**

Structured text substitution. Like C macros but syntax-aware.

```logos
## Template: with_logging (name: Text, body: Block):
    Print "Entering " ++ name.
    body
    Print "Exiting " ++ name.

## To process (x: Int) -> Int with logging:
    Return x * 2.
```

Templates expand before parsing. The template system operates on token streams, not AST nodes.

Integration: New pre-parser phase in `crates/logicaffeine_language/`. Templates are parsed from `## Template:` blocks during discovery. Expansion happens between tokenization and full parsing.

**Approach B: AST Macros (3-6 months)**

Programs manipulate AST nodes as values. Requires exposing the AST as a LOGOS data type.

```logos
## Macro: timed (body: Code) -> Code:
    Return Quote:
        Let start be now().
        Splice body.
        Let elapsed be since(start).
        Print "Took " ++ elapsed ++ "ms".
```

`Code` is a type representing LOGOS syntax. `Quote` captures syntax. `Splice` inserts code. This is Lisp's quasiquoting.

Integration: New types in `ast/stmt.rs` — `Stmt::Quote`, `Stmt::Splice`, `Expr::Code`. The macro expander runs after parsing but before analysis. Need a LOGOS-level representation of the AST that's simpler than the Rust-level one.

**Approach C: English-Native Metaprogramming (the LogicAffeine approach, 6+ months)**

Macros described in natural language. The macro system IS pattern matching on English descriptions.

```logos
## Rule: whenever a function is marked "measured":
    Record the start time before the body.
    Record the elapsed time after the body.
    Print the function name and elapsed time.

## To sort (items: Seq of Int) -> Seq of Int is measured:
    ...
```

The "Rule" blocks are parsed into transformation specifications. The compiler applies matching rules to function definitions. This is the most aligned with LogicAffeine's philosophy — metaprogramming through English descriptions.

Integration: New `Stmt::Rule` variant. New transformation pass in the compiler. Rule matching against function metadata (annotations, types, names).

### Integration Points

- `ast/stmt.rs` — macro/template/rule variants
- `parser/mod.rs` — parse new block types
- New compiler pass (pre-analysis or pre-codegen)
- `codegen.rs` — emit expanded code
- Potentially `crates/logicaffeine_language/src/analysis/discovery.rs` — discover templates/rules early

---

## 10. Jepsen — Distributed Correctness Testing

### The Idea

Jepsen is Kyle Kingsbury's framework for breaking distributed systems. It starts your system, partitions the network, kills nodes, introduces clock skew, then checks if the system maintained its consistency guarantees. It has broken Kafka, MongoDB, Redis, CockroachDB, and nearly every distributed database.

### What We Have Today

A rich distributed systems story:
- 12+ CRDT types in `logicaffeine_data` (ORSet, ORMap, GCounter, PNCounter, RGA, YATA, etc.)
- Delta-state replication protocol (DeltaCrdt, DeltaBuffer traits)
- Vector clocks for causality (VClock)
- Persistent journals (`Mount`) in `logicaffeine_system`
- GossipSub pub/sub (`Sync`) via libp2p
- Agent model (`Spawn`, `SendMessage`, `AwaitMessage`)

CRDTs are *mathematically* convergent. But the system layer — journal persistence, GossipSub delivery, Mount/Sync orchestration — could have bugs. The delta-state protocol, the journal replay, the conflict resolution sequencing — these are where bugs hide.

### What It Unlocks

- Confidence that the distributed features actually work under failure
- Marketing: "Jepsen-tested CRDTs" is a powerful claim
- Bug discovery in journal persistence, delta shipping, conflict resolution
- Regression testing for the system layer

### What It Takes to Build

**Phase 1: Chaos Testing Primitives (medium)**

Add test-only network fault injection to the system crate:

```rust
// In logicaffeine_system, behind #[cfg(test)]
pub struct ChaosNetwork {
    drop_rate: f64,
    delay_range: Range<Duration>,
    partition_sets: Vec<HashSet<ReplicaId>>,
}
```

Integration: `crates/logicaffeine_system/` — injectable network layer for testing. The real libp2p layer gets wrapped with a chaos proxy.

**Phase 2: History Verification**

After a chaos test run, check that all replicas converged to the same state:

```rust
fn verify_convergence(replicas: &[Replica]) -> Result<(), JepsenError> {
    let states: Vec<_> = replicas.iter().map(|r| r.state()).collect();
    assert!(states.windows(2).all(|w| w[0] == w[1]));
}
```

Integration: `crates/logicaffeine_tests/` — new test category for distributed correctness.

**Phase 3: Language-Level Chaos Testing**

```logos
## Test: counter convergence under partition:
    Spawn 5 agents each with a shared GCounter.
    Partition agents into groups [1,2] and [3,4,5].
    Each agent increments 100 times.
    Heal partition.
    Wait for sync.
    Assert all agents agree on count 500.
```

This is a LOGOS-level test framework that uses the language's own primitives. The test runner injects faults between agent communications.

### Integration Points

- `crates/logicaffeine_system/` — chaos network layer, fault injection
- `crates/logicaffeine_data/` — convergence verification utilities
- `crates/logicaffeine_tests/` — Jepsen-style test suite
- `apps/logicaffeine_cli/src/cli.rs` — `largo test --chaos` command

---

## 11. Susan Haack — Philosophy of Logics

### The Idea

Haack argues there's no single "correct" logic. Different reasoning contexts demand different formal systems. Classical logic isn't wrong, but neither is intuitionistic logic or relevant logic or paraconsistent logic. The choice of logic should be a pragmatic decision, not a metaphysical one.

### What We Have Today

LogicAffeine already implements logical pluralism:
- **Classical FOL**: ∀, ∃, ∧, ∨, →, ¬ with standard truth-functional semantics
- **Modal logic**: □ (necessity), ◇ (possibility) with Kripke world semantics. Three domains: alethic, deontic, epistemic
- **Temporal logic**: Prior's P (past) and F (future) operators
- **Aspectual logic**: Progressive, Perfect, Habitual, Iterative operators
- **Event semantics**: Neo-Davidsonian events with thematic roles
- **Intensional logic**: ^P (intension), opaque contexts for beliefs/desires
- **Generalized quantifiers**: Most, Few, Many, Cardinal(n), AtLeast(n), AtMost(n)
- **Presupposition logic**: Assertion/presupposition split

The `ModalVector` type (domain + force + flavor) explicitly represents the *choice* of modal system.

### What It Unlocks (with extension)

- **Explicit logic selection**: "Using intuitionistic logic: ..." — the user chooses which logic governs a block
- **Paraconsistent CRDT resolution**: When CRDTs have conflicting states, reason about them without explosion (in classical logic, a contradiction entails everything)
- **Relevance logic for causation**: Your `Causal { effect, cause }` AST node could be backed by a relevant implication (A→B only holds if A is relevant to B, not just whenever A is false)
- **Multi-logic verification**: Your Z3 verifier could check properties under different logical assumptions

### What It Takes to Build

**For the declarative mode**: Already mostly there. The different logics coexist in the AST. Extension would be allowing the user to specify which logic to use for verification:

```logos
Assert using relevant logic that the cause implies the effect.
Assert using intuitionistic logic that the proof is constructive.
```

Integration: `crates/logicaffeine_verify/src/solver.rs` — different Z3 encoding strategies per logic. `ast/logic.rs` — annotation on assertions specifying the logic.

**For the imperative mode**: Paraconsistent reasoning during CRDT conflict resolution. When an MVRegister has multiple values, don't force a single winner — reason about the superposition of states.

Integration: `crates/logicaffeine_data/` — paraconsistent merge strategies. `codegen.rs` — emit conflict resolution code that handles contradictions gracefully.

---

## 12. Godbolt — Compiler Explorer

### The Idea

See your source code and its compiled output side by side, live. Type on the left, see assembly on the right. Instant feedback on what the compiler does with your code.

### What We Have Today

- `logicaffeine_web` — a full Dioxus-based web IDE, already WASM-compiled
- `compile_to_rust()` — produces readable Rust source as intermediate output
- `transpile.rs` — produces FOL in Unicode, LaTeX, ASCII, and Kripke formats
- `diagnostic.rs` — translates rustc errors back to LOGOS source locations

We're one step away from a LogicAffeine Compiler Explorer.

### What It Unlocks

- Killer demo: "Watch English become logic and code in real time"
- Educational tool: see how natural language maps to formal logic
- Debugging: understand what the compiler does with your code
- Marketing: the visual pipeline is immediately impressive

### What It Takes to Build

**Phase 1: Triple-Pane View in Web IDE**

```
┌──────────────────┬──────────────────┬──────────────────┐
│    LOGOS Input    │   FOL Output     │   Rust Output    │
│                  │                  │                  │
│ Every cat sleeps.│ ∀x(Cat(x) →     │ // Generated     │
│                  │    Sleep(x))     │ fn main() {      │
│ Let x be 5.     │                  │   let x = 5;     │
│ Set x to x + 1. │                  │   x = x + 1;     │
│                  │                  │ }                │
└──────────────────┴──────────────────┴──────────────────┘
```

All three panes update live as the user types. The left pane is the editor. The middle shows the FOL transpilation (declarative) or nothing (imperative). The right shows the generated Rust.

Integration: `apps/logicaffeine_web/` — add split-pane layout. Call `compile_to_rust()` and `transpile()` on each keystroke (debounced). Both functions are already WASM-compatible.

**Phase 2: Source Mapping**

Click a line in LOGOS, highlight the corresponding FOL and Rust. Click a Rust line, highlight the originating LOGOS. This requires source maps through the pipeline.

Integration: `codegen.rs` — emit source location comments in generated Rust. `transpile.rs` — track source spans through FOL generation. Web IDE — bidirectional highlighting from source maps.

**Phase 3: Chain to Godbolt**

Add a fourth pane that calls Godbolt's API to show the final x86/ARM/WASM assembly of the generated Rust. The full pipeline visualization: English → Logic → Rust → Assembly.

---

## 13. Fully Homomorphic Neural Networks

### The Idea

Run computation on encrypted data. The data owner never reveals their data. The model owner never reveals their weights. FHE (Fully Homomorphic Encryption) allows arbitrary computation on ciphertexts — the result, when decrypted, is the same as if you'd computed on the plaintext.

### What We Have Today

Security primitives (`Check` statements), capability-based access control, and CRDTs that handle distributed state. No encryption beyond what TLS provides for networking.

### What It Unlocks

- Privacy-preserving distributed computation: agents compute on each other's encrypted data
- Encrypted CRDTs: counters and sets that converge without revealing their contents
- Compliance: computation that provably never sees raw data (GDPR, HIPAA)

### What It Takes to Build

This is the most research-frontier item. FHE libraries exist (TFHE-rs, concrete) but are expensive (1000x+ overhead for general computation). However:

**Specific opportunity: Encrypted CRDTs**

Your CRDTs involve simple operations — addition (GCounter, PNCounter), comparison (LWWRegister), set union (ORSet). These are FHE-friendly operations. A GCounter over encrypted values only needs encrypted addition, which is the cheapest FHE operation.

```logos
Inside an encrypted zone:
    Increase counter's value by 1.  ## Operates on ciphertext
```

Integration: `crates/logicaffeine_data/` — encrypted variants of CRDTs (e.g., `EncryptedGCounter`). `codegen.rs` — emit FHE library calls inside encrypted zones. `logicaffeine_system` — key management, encrypted network transport.

### Realistic Timeline

Encrypted CRDTs for simple operations (counters, registers): 3-6 months with TFHE-rs. General-purpose FHE for arbitrary LOGOS programs: years of research.

---

## 14. Unison — Content-Addressed Code

### The Idea

Every definition is stored by its hash, not its name. If the implementation doesn't change, the hash doesn't change. Compiled artifacts are cached by hash — if the hash exists in the cache, don't recompile. Names are just human-readable labels attached to hashes. This eliminates builds, enables distributed code sharing, and makes incremental compilation trivial.

### What We Have Today

- Symbol interning (`Interner`) gives canonical representations to strings
- SHA-256 in `logicaffeine_system` for journal integrity
- `publish`/registry system for distributing packages
- No content addressing of individual definitions

### What It Unlocks

- **Instant incremental compilation**: Changed function? Only recompile that one function's hash. Everything else is cached.
- **Distributed code sharing**: Import a function by hash. The runtime fetches it from the registry. No package manager needed.
- **Fearless refactoring**: Rename a function? The hash doesn't change (it's based on the body, not the name). All dependents still work.
- **Deduplication**: Two packages with identical functions (different names) share the same compiled artifact.

### What It Takes to Build

**Phase 1: Definition Hashing**

After parsing, compute a stable hash of each function/struct/enum definition:

```rust
fn hash_definition(def: &FunctionDef, interner: &Interner) -> ContentHash {
    let mut hasher = Sha256::new();
    hasher.update(canonical_form(def, interner));
    ContentHash(hasher.finalize())
}
```

"Canonical form" strips names and formatting, keeping only structure and semantics.

Integration: New module `crates/logicaffeine_compile/src/content_hash.rs`. Runs after parsing, before codegen.

**Phase 2: Compilation Cache**

Map `ContentHash → generated-rust-source`. Before codegen for a function, check the cache. If hit, skip codegen.

Integration: Cache stored in `~/.config/logos/cache/` or project-local `.logos-cache/`. `compile.rs` — insert cache lookup before codegen. `codegen.rs` — write to cache after generating.

**Phase 3: Distributed Definitions**

```logos
## Import hash "a1b2c3d4" as sort_function.
```

The registry serves individual definitions by hash, not just whole packages.

Integration: `apps/logicaffeine_cli/src/project/registry.rs` — definition-level fetch. `crates/logicaffeine_compile/src/compile.rs` — resolve hash imports during compilation.

---

## 15. Inigo Quilez — Shader Math & Creative Coding

### The Idea

Quilez demonstrates that pure mathematical functions can generate stunning visual output. Signed distance functions, procedural generation, ray marching — all expressed as elegant, composable math.

### What We Have Today

Numeric types (`Int`, `Nat`, `Real`), arithmetic operations, function definitions. No vector types, no GPU codegen, no visual output.

### What It Unlocks

- LOGOS as a creative coding tool: describe visual algorithms in English
- GPU acceleration for compute-heavy LOGOS programs
- A dramatically different demo from the typical PL showcase

### What It Takes to Build

**Phase 1: Vector/Matrix Primitives**

```logos
## A Vec3 has (portable):
    An x: Real.
    A y: Real.
    A z: Real.

## To normalize (v: Vec3) -> Vec3:
    Let len be sqrt(v's x * v's x + v's y * v's y + v's z * v's z).
    Return a new Vec3 with x (v's x / len) and y (v's y / len) and z (v's z / len).
```

These could be built-in types with optimized codegen (SIMD operations).

**Phase 2: Shader Codegen Backend**

Instead of Rust, emit GLSL or WGSL for functions annotated as shaders:

```logos
## To distance_from_sphere (p: Vec3) and (center: Vec3) and (radius: Real) -> Real is a shader:
    Return length(p - center) - radius.
```

Integration: New codegen backend `crates/logicaffeine_compile/src/codegen_wgsl.rs` alongside the existing `codegen.rs`. Function-level backend selection based on annotations.

---

## 16. Laziness

### The Idea

"Super eager but lazy." Evaluate eagerly when you can (super-compilation), but support lazy evaluation for things that shouldn't be computed until needed. Do as much as possible as early as possible, but never do work that's never needed.

### What We Have Today

Strict/eager evaluation everywhere. All expressions evaluate immediately. No lazy evaluation, no thunks, no generators.

### What It Unlocks

- Infinite data structures (lazy lists, streams)
- Demand-driven computation (only compute what's actually used)
- Better performance for pipelines (no intermediate collections)
- Natural fit with super-compilation: known values → eager, unknown → lazy

### What It Takes to Build

**Approach: Lazy Sequences as Rust Iterators**

```logos
Let naturals be a lazy Seq from 1 counting up.
Let evens be naturals where it is even.
Let first_10 be the first 10 of evens.  ## Only evaluates 10 elements
```

Codegen emits Rust iterators:

```rust
let naturals = (1..).into_iter();
let evens = naturals.filter(|x| x % 2 == 0);
let first_10: Vec<_> = evens.take(10).collect();
```

Integration:
- `ast/stmt.rs` — `Expr::LazySeq`, `Expr::Filter`, `Expr::Take`
- `parser/mod.rs` — parse "a lazy Seq", "where it is", "the first N of"
- `codegen.rs` — emit iterator chains

---

## 17. HVM2 & Interaction Nets

### The Idea

HVM2 (Higher-Order Virtual Machine) uses interaction nets — a graph-based execution model based on Lafont's interaction combinators and Lamping's abstract algorithm. It achieves optimal sharing: every computation is done exactly once, no matter how many times the result is needed. It's inherently parallel — independent reductions happen simultaneously.

### What We Have Today

Lambda calculus in the AST: `LogicExpr::Lambda { variable, body }`, `LogicExpr::App { function, argument }`. These are the exact constructs that interaction nets encode. The declarative mode already builds lambda terms.

### What It Unlocks

- Optimal reduction: no redundant computation
- Automatic parallelism: the runtime parallelizes without user annotation
- Natural fit for the declarative mode: lambda calculus IS what interaction nets evaluate
- A path to making super-compilation practical

### What It Takes to Build

This is deep research territory. A realistic path:

**Phase 1: Study and Prototype (months)**

Implement a minimal interaction net evaluator for the logic fragment. Take `LogicExpr::Lambda` and `LogicExpr::App` nodes, convert to interaction net nodes, reduce.

Integration: New crate `crates/logicaffeine_inet/` or a module in `logicaffeine_proof/`. Operates on the logic AST, not the imperative AST.

**Phase 2: Connect to Proof Engine**

The proof engine (`logicaffeine_proof`) does backward-chaining. Interaction nets could provide an alternative evaluation strategy for proof search.

**Phase 3: Optimizing Backend**

Use interaction nets as an optimization pass: convert pure functions to interaction nets, reduce, convert back to AST. This is super-compilation via graph rewriting.

---

## 18. Esperanto — Universal Language

### The Idea

LogicAffeine as the Esperanto of programming — a constructed language designed to be universally accessible. Not just English-to-code, but potentially any-natural-language-to-code.

### What We Have Today

The lexicon system (`assets/lexicon.json`) separates vocabulary from grammar. 56 lookup functions map English words to linguistic properties. The parser handles English grammar specifically, but the lexical layer is pluggable.

### What It Unlocks

- Multilingual programming: write LOGOS in French, Spanish, Mandarin
- True universal accessibility: programming in your native language
- The FOL output is already language-independent (∀x(P(x)) is ∀x(P(x)) in any language)

### What It Takes to Build

**Phase 1: Lexicon Abstraction**

The lexicon is already JSON-based and separate from the parser. Create additional lexicon files:

```
assets/lexicon_en.json  (current)
assets/lexicon_es.json  (Spanish)
assets/lexicon_fr.json  (French)
```

Integration: `crates/logicaffeine_lexicon/` — load lexicon by language code. The `dynamic-lexicon` feature flag already supports runtime lexicon loading.

**Phase 2: Grammar Parameterization**

Different languages have different word orders (SVO, SOV, VSO). The parser would need grammar rules parameterized by language:

- English: "Every cat sleeps" (SVO)
- Japanese: "Every cat sleeps" would be "Subete no neko wa nemuru" (SOV)

This is a major undertaking — essentially a different parser per language family. A more realistic approach: support a controlled subset of natural language where word order differences are minimal.

---

## 19. The Glue Language Vision

### The Idea

The language should be excellent at connecting disparate systems. Not just writing programs, but wiring services, formats, protocols, and runtimes together.

### What We Have Today

- **Logic glue**: English → FOL bridges natural language and formal reasoning
- **Distributed glue**: CRDTs + GossipSub + Agents bridge distributed systems
- **Data glue**: Mount/Sync/Journal bridge persistence and computation
- **WASM glue**: runs in browsers, servers, and embedded contexts

### What's Missing

- FFI (sections 1-2 cover this)
- Format conversion (JSON, YAML, TOML, CSV, XML)
- HTTP client/server
- Database connectors
- Message queue integration (Kafka, RabbitMQ, NATS)

### What It Takes to Build

Most of this is unlocked by Escape Hatches + Dependency Declarations. Once you can call Rust crates, you get:
- `serde_json` for JSON
- `reqwest` for HTTP
- `sqlx` for databases
- `rdkafka` for Kafka
- Everything else in the Rust ecosystem

The language-level additions would be sugar:

```logos
Read data from JSON "config.json" into settings.
Send a POST request to "https://api.example.com/data" with body payload.
Query "SELECT * FROM users WHERE age > 21" into results.
```

These map to native function declarations (Tier 2 FFI) backed by Rust crate calls.

Integration: A standard library of native function declarations for common operations. `crates/logicaffeine_compile/src/stdlib/` — pre-declared native functions for JSON, HTTP, SQL, etc.

---

## 20. Eliminating the Compile/Interpret/Run Distinction

### The Idea

There shouldn't be hard phases. The compiler/interpreter/runtime merge into one continuous evaluation that does as much as it can whenever it can. Input arrives → evaluate what you can → emit code for what you can't → more input → evaluate more → repeat.

This is the philosophical culmination of super-compilation (section 3), laziness (section 16), and partial evaluation. It's JIT-like but inverted: instead of "compile hot paths at runtime," it's "evaluate everything you can at compile time and only emit code for the truly dynamic parts."

### What We Have Today

Hard-phased pipeline: Lex → Parse → Analyze → Codegen → Compile → Run. Each stage completes before the next begins. But both a compiler and interpreter exist in the same crate.

### What It Unlocks

- The REPL and the compiler are the same thing
- Type definitions? Evaluated immediately. Functions? Compiled if needed, evaluated if possible.
- Loading a configuration file at compile time and specializing the program for that config — automatically
- Multi-stage programs where some parts run at compile time and others at runtime, determined by information availability rather than programmer annotation

### What It Takes to Build

This is the deepest architectural change and the one that would most differentiate LogicAffeine from other languages.

**The Architecture: A Unified Evaluator**

```
┌─────────────────────────────────────────────┐
│               Unified Evaluator             │
│                                             │
│  Input arrives (source, data, arguments)    │
│        │                                    │
│        ▼                                    │
│  ┌───────────┐                             │
│  │ Can I     │──yes──▶ Evaluate & cache    │
│  │ evaluate? │         result              │
│  └───────────┘                             │
│        │ no                                │
│        ▼                                    │
│  ┌───────────┐                             │
│  │ Can I     │──yes──▶ Specialize & emit   │
│  │ partially │         residual code       │
│  │ evaluate? │                             │
│  └───────────┘                             │
│        │ no                                │
│        ▼                                    │
│  Emit general code                         │
│                                             │
│  More input arrives → re-evaluate          │
└─────────────────────────────────────────────┘
```

The interpreter becomes the "Can I evaluate?" check. The codegen becomes the "emit code" fallback. Partial evaluation bridges the two.

**Stage 1**: Constant folding (section 3, stage 1) — the simplest form of compile-time evaluation.

**Stage 2**: Pure function evaluation at compile time (section 3, stage 4) — use the interpreter as an oracle during compilation.

**Stage 3**: The unified evaluator — a new component that orchestrates the interpreter and codegen, deciding at each AST node whether to evaluate or emit.

Integration: `crates/logicaffeine_compile/src/unified.rs` (new) — the unified evaluator that dispatches between `interpreter.rs` and `codegen.rs` at the expression/statement level.

---

## Dependency Graph

Some of these features enable others. Here's the dependency structure:

```
Escape Hatches (1) ──────────────────────────────────────────────┐
    │                                                             │
    ├──▶ FFI (2) ──▶ Glue Language (19) ──▶ Stable ABI (4)      │
    │                                                             │
    └──▶ Dependency Declarations                                  │
                                                                  │
Fast Compilation (7) ◀── Content-Addressed Code (14)             │
    │                                                             │
    └──▶ Interpreter Mode ──▶ Unified Evaluator (20)             │
                                    │                             │
                    Constant Folding ┘                            │
                         │                                        │
                         ▼                                        │
                Partial Evaluation (3) ◀── Laziness (16)         │
                         │                                        │
                         ▼                                        │
                Super-Compilation ◀── HVM2/Interaction Nets (17) │
                                                                  │
Structural Subtyping (5) ── independent                          │
                                                                  │
Cross-Compilation (6) ◀── Escape Hatches (1)                     │
                                                                  │
Macros (9) ── independent but enhanced by ──────────────────────┘

Jepsen (10) ◀── CRDTs already exist

Godbolt Explorer (12) ◀── Web IDE already exists

Encrypted CRDTs (13) ── independent research

Esperanto (18) ◀── Lexicon system already exists

Susan Haack (11) ◀── Multiple logics already exist

Shader Math (15) ── independent
```

**Critical path**: Escape Hatches → FFI → Glue Language. Everything else can be pursued in parallel.

---

## Effort Estimates

| Feature | Effort | Prerequisite |
|---------|--------|-------------|
| Escape Hatches | 2-4 weeks | None |
| FFI Tier 1 (Dependencies) | 1-2 weeks | None |
| FFI Tier 2 (Native Functions) | 2-3 weeks | Tier 1 |
| FFI Tier 3 (Bidirectional) | 3-4 weeks | Tier 2 |
| Constant Folding | 1-2 weeks | None |
| Dead Code Elimination | 1-2 weeks | Constant Folding |
| Interpreter CLI Mode | 1 week | None |
| Godbolt Triple-Pane | 2-3 weeks | None |
| Structural Subtyping | 4-6 weeks | None |
| Content-Addressed Hashing | 3-4 weeks | None |
| Compilation Cache | 2-3 weeks | Content Hashing |
| `--target wasm` | 1-2 weeks | None |
| Stable ABI (Cap'n Proto) | 4-8 weeks | None |
| Template Macros | 4-6 weeks | None |
| AST Macros | 3-6 months | Template Macros |
| Jepsen Test Framework | 6-8 weeks | None |
| Lazy Sequences | 3-4 weeks | None |
| Partial Evaluation | 3-6 months | Constant Folding + DCE |
| Unified Evaluator | 6-12 months | Partial Evaluation |
| HVM2 / Interaction Nets | 6-12 months | Research |
| Encrypted CRDTs | 3-6 months | Research |
| Multilingual Lexicons | 4-8 weeks per language | None |
| Shader Codegen | 3-6 months | Vector types |
| Rc-based Memory | 2-4 weeks | None |

---

## What This All Means

These ideas cluster around a central vision: **LogicAffeine as the universal intermediary language**. English in, anything out. The escape hatch makes it practical today. The FFI makes it useful tomorrow. The super-compilation makes it fast. The stable ABI makes it interoperable. The macros make it extensible. The Godbolt view makes it understandable. The Jepsen testing makes it trustworthy.

The compiler-interpreter unification (section 20) is the most architecturally transformative idea. It turns LogicAffeine from "a language that compiles" to "an intelligent evaluator that does whatever is most efficient with the information it has." That's the Esperanto connection — not just universally readable input, but universally adaptive execution.
