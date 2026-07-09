# TYPESTATE.md — Typestate Pipeline Design for LogicAffeine

This document records the design analysis of a typestate-based optimizer pipeline that
solves four architectural problems simultaneously: pass ordering, plugin safety,
refinement type unification, and specialization boundary management. It also includes
the interview/question log that led to the design.

---

## Table of Contents

1. [Pass Ordering: What We're Leaving on the Table](#1-pass-ordering)
2. [Plugin Architecture](#2-plugin-architecture)
3. [Refinement Types + Abstract Interpretation Unification](#3-refinement-unification)
4. [Specialization Boundaries](#4-specialization-boundaries)
5. [Typestate Pipeline: Encoding Pass Ordering in the Type System](#5-typestate-pipeline)
6. [Summary Table](#summary)
7. [Interview Log](#interview-log)

---

## 1. Pass Ordering: What We're Leaving on the Table

### The Claim (Invariant 3)
> "Each optimization must work correctly regardless of whether other optimizations ran before it."

**Correctness is order-independent. Quality is not.** The pipeline in `optimize/mod.rs:50-88` is:

```
FIXPOINT (x8): fold → propagate → PE
LINEAR:        CTFE → fold → CSE → LICM → closed-form → deforest → abstract-interp → DCE → supercompile
```

The fixpoint loop handles mutually-enabling passes correctly. The problem is the **linear chain** — each pass runs exactly once, and later passes create opportunities for earlier ones that are never picked up.

### Concrete Missed Optimization Chains

**1. Deforestation → CSE/LICM (Severe)**

```logos
Let temp be a new Seq of Int.
Repeat for x in source: Push (x * 2) to temp.
Let sum be 0.
Repeat for y in temp: Set sum to sum + y.
```

After deforestation fuses into a single loop, new `Let` bindings appear inside the fused body. These are candidates for LICM hoisting — but LICM already ran (line 77). The `is_worth_hoisting()` function in `licm.rs:81-91` would fire on `BinaryOp` expressions, but it never gets the chance.

**2. Closed-Form → CSE (Significant)**

Closed-form replaces `sum += i` loops with `n * (n + 1) / 2`. This creates new binary-op trees that CSE (`gvn.rs`) could deduplicate if the same Gauss formula appears twice. But CSE ran before closed-form.

**3. Abstract Interpretation → Fold/Propagate (Moderate)**

Abstract interp narrows `x` to `[11, ∞)` inside `if x > 10:` branches. Fold could use this to simplify expressions involving `x`, but fold already ran. The range information is computed too late.

**4. Supercompiler's Intentional Blind Spots (Architectural)**

The supercompiler deliberately avoids transforming certain patterns to preserve codegen peephole targets:

- `Expr::Index` and `Expr::Slice` → **pass-through** (lines 479-483, `supercompile.rs`): "preserve variable names for codegen peephole patterns"
- `Expr::Call` → **not inlined at AST level** (lines 462-474): inner calls like `foo(factorial(5))` don't pre-evaluate the nested call
- Loop widening → **removes all modified vars from store** (lines 619-646): no MSG (Most Specific Generalization), no precise widening. The embedding check is computed but **never used** — dead code.

This means the supercompiler does NOT catch what earlier passes missed. It's a cleanup pass, not a universal subsumer.

### What "~5% on the table" Actually Means

| Earlier Pass | Later Pass Creates | What's Missed | Severity |
|---|---|---|---|
| CSE | Closed-form binary-op trees | Duplicate Gauss formulas | Significant |
| LICM | Deforestation creates new Lets | Hoistable expressions in fused loops | Severe |
| Fold/Propagate | Abstract-interp narrows ranges | Branch simplification with known ranges | Moderate |
| CSE/LICM | Supercompile constants | Constants from driving not deduplicated/hoisted | Low |

### Fixes (Not Implemented)

1. **Re-run after structural transforms:** After deforestation, re-run CSE + LICM. After closed-form, re-run CSE. Cost: ~30% more compile time, captures ~5% more optimizations.
2. **Second fixpoint on the whole chain:** Wrap all passes in a convergence loop. Diminishing returns but catches everything.
3. **Extend supercompiler scope:** Remove Index/Slice/Call pass-throughs. Let the supercompiler fully drive, then re-run peephole patterns in codegen.
4. **Fix the dead code in supercompile.rs:** The embedding check at line 646 computes but never acts. Wire it to MSG-based widening.

---

## 2. Plugin Architecture

### Current State
Zero plugin infrastructure. The compiler is monolithic — all passes are hardcoded Rust functions called in sequence in `optimize/mod.rs`.

### Trait-Based Pass System

```rust
pub trait OptimizationPass: Send + Sync {
    fn name(&self) -> &str;
    fn run<'a>(&self, stmts: Vec<Stmt<'a>>, ctx: &mut OptContext<'a>) -> Vec<Stmt<'a>>;
    fn depends_on(&self) -> &[&str];      // passes that must run before this one
    fn conflicts_with(&self) -> &[&str];  // passes incompatible with this one
    fn is_fixpoint(&self) -> bool;        // should this participate in the fixpoint loop?
}

pub struct OptContext<'a> {
    pub expr_arena: &'a Arena<Expr<'a>>,
    pub stmt_arena: &'a Arena<Stmt<'a>>,
    pub interner: &mut Interner,
    pub stats: PassStats,           // timing, change counts
    pub config: OptConfig,          // tier, disabled passes
}

pub struct PassStats {
    pub duration: Duration,
    pub changes: usize,
    pub stmts_before: usize,
    pub stmts_after: usize,
}
```

Each existing pass wraps into this trait. The pipeline becomes a topological sort:

```rust
pub struct Pipeline {
    passes: Vec<Box<dyn OptimizationPass>>,
    fixpoint_group: Vec<usize>,  // indices of passes in the fixpoint loop
}

impl Pipeline {
    pub fn run<'a>(&self, stmts: Vec<Stmt<'a>>, ctx: &mut OptContext<'a>) -> Vec<Stmt<'a>> {
        // 1. Run fixpoint group until stable
        // 2. Run remaining passes in topological order
        // 3. If config.double_fixpoint, repeat step 2
    }
}
```

### Tiered Compilation

| Mode | Passes | Compile Speed | Optimization |
|------|--------|--------------|--------------|
| `--debug` / `-O0` | fold only | ~10ms | Minimal |
| `--default` / `-O1` | fold + propagate + PE + DCE | ~50ms | Good |
| `--release` / `-O2` | full pipeline (current) | ~200ms | Full |
| `--aggressive` / `-O3` | full pipeline + second fixpoint | ~400ms | Maximum |
| `--profile` | full + per-pass timing output | ~250ms | Full + stats |

### Hot-Reload Architecture

LOGOS compiles to Rust, which compiles to native code. Hot-reload requires:

1. **Function-level compilation units:** Each LOGOS function compiles to a separate Rust module/crate
2. **Dynamic dispatch at boundaries:** Function calls go through a vtable or function pointer table
3. **File watcher + incremental recompile:** `notify` crate watches `.logos` files, recompiles changed functions
4. **Library swap:** Compile changed function to `.dylib`, `dlopen` + replace function pointer

```
largo watch src/      # starts file watcher
  → detects change to compute.logos
  → recompiles compute() only
  → swaps libcompute.dylib
  → running program picks up new implementation on next call
```

**Key constraint:** This only works for leaf functions. Changing a function's signature requires recompiling all callers.

### Runtime Plugin Distribution

**WASM plugins (recommended for distribution):**

```
largo plugin install optimizer-cache-tiler
largo build --plugins=cache-tiler
```

The plugin runs inside a WASM sandbox (wasmtime). It receives a serialized AST, transforms it, returns the result. The serialization boundary is the cost — but it buys sandboxing and cross-platform compatibility.

**Plugin manifest:**
```toml
[plugin]
name = "cache-tiler"
version = "0.1.0"
pipeline_stage = "after:licm"      # where in the pipeline
replaces = []                       # passes it supersedes
requires = ["abstract-interp"]      # passes it depends on
min_compiler = "0.9.0"             # version compatibility
```

**Serializable IR format:** The AST needs a stable serialization format. Options:
- **Protobuf/FlatBuffers:** Typed, fast, versioned schemas. Best for production.
- **JSON:** Easy debugging, slow. Good for development.
- **Custom binary:** Fastest, but maintenance burden.

The IR should be the **post-parse, pre-optimize AST** — `Vec<Stmt>` serialized with all expression trees intact. This is the natural plugin boundary because it's the common input/output type for all passes.

**dylib plugins (for power users):**

Same trait interface but loaded via `dlopen`. No serialization overhead. ABI compatibility guaranteed by pinning the `Stmt`/`Expr` types to a stable layout (or using `#[repr(C)]`).

---

## 3. Refinement Types + Abstract Interpretation Unification

### Three Disconnected Systems

**System A — RefinementContext** (`codegen/context.rs`):
```rust
struct RefinementContext<'a> {
    scopes: Vec<HashMap<Symbol, (Symbol, &'a LogicExpr<'a>)>>,  // var → (bound_var, predicate)
    variable_types: HashMap<Symbol, String>,  // types as STRINGS (not structured)
    live_vars_after: Option<HashSet<Symbol>>,
}
```
- Stores FOL predicates from user annotations (`x: { it > 0 }`)
- Emits `debug_assert!()` on mutation
- **Not used for optimization.** Purely runtime safety.
- Type info stored as strings (`"Vec<i64>"`) — codegen does `t.contains("HashMap")` instead of pattern matching on `LogosType`

**System B — AbstractInterpreter** (`optimize/abstract_interp.rs`):
```rust
struct AbstractState {
    vars: HashMap<Symbol, Interval>,     // var → numeric range [lo, hi]
    lengths: HashMap<Symbol, Interval>,  // collection → length range
}
```
- Forward analysis computing `Interval { lo: Bound, hi: Bound }`
- Branch narrowing: `if x > 5` → `x in [6, +inf)` in then-branch
- Collection length tracking
- **Results discarded after DCE.** Not persisted to codegen.

**System C — Z3 Verification** (`verification.rs`):
```rust
struct VerificationPass<'a> {
    session: VerificationSession,   // Z3 solver
    interner: &'a Interner,
}
```
- Maps LOGOS refinement types to Z3 sorts
- Verifies assertions against accumulated constraints
- **Completely isolated.** Runs independently, results not fed to optimizer or codegen.
- Feature-gated + license-gated

### What's Not Flowing

| From | To | Missing Data | Concrete Impact |
|---|---|---|---|
| AbstractState ranges | Peephole bounds elision | Index safety proofs | OPT-4 only matches one syntactic pattern; abstract-interp could prove bounds for 12+ benchmarks |
| RefinementContext predicates | AbstractState initialization | `x: { it > 0 }` → `x in [1, +inf)` | Range analysis starts from `[-inf, +inf]` even when user declared narrower type |
| AbstractState ranges | RefinementContext assertions | Statically-proven predicates | `Let x = 10 { it > 0 }. Set x to 5.` emits runtime `debug_assert!(5 > 0)` instead of proving it statically |
| Z3 proofs | Peephole | Verified safe accesses | Could emit `unsafe { get_unchecked }` with Z3-backed proof |
| TypeEnv (structured) | RefinementContext | `LogosType` instead of strings | String-based type checks are fragile and incomplete |

### Concrete Example: Three Missed Optimizations in One Program

```logos
Let items be a new Seq of Int.
Repeat for i from 1 to 100: Push 0 to items.
Let x be 50 { it > 0 }.
Let val be item x of items.
Set x to 25.
```

| System | Observes | Misses |
|---|---|---|
| **AbstractState** | `items.length = [100, 100]`, `x = [50, 50]` then `[25, 25]` | Doesn't tell codegen that `item 50 of items` is safe |
| **RefinementContext** | `x: { it > 0 }` predicate | Emits `debug_assert!(25 > 0)` at runtime even though `25 > 0` is trivially true |
| **Z3** | Could prove `50 in [1, 100]` → bounds safe | Never runs during optimization; only runs if user opts into verification |
| **Peephole** | Syntactic counter check for OPT-4 | `x` isn't a loop counter, so bounds elision doesn't fire |

**With unification:**
1. RefinementContext feeds `x > 0` to AbstractState → `x in [1, +inf)` (start narrower)
2. AbstractState proves `x in [50, 50]` at index site + `items.length in [100, 100]` → **bounds safe**
3. Codegen checks `statically_proven` flag → skips `debug_assert!` and emits unchecked index
4. Z3 is optional escalation for cases abstract-interp can't handle

### Unification Design

```rust
/// Shared constraint representation flowing through the pipeline
pub struct ConstraintEnv {
    /// Numeric ranges from abstract interpretation
    ranges: HashMap<Symbol, Interval>,
    /// Collection lengths from abstract interpretation
    lengths: HashMap<Symbol, Interval>,
    /// User-declared refinement predicates
    refinements: HashMap<Symbol, Vec<LogicExpr>>,
    /// Statically proven safe accesses (index, collection)
    proven_safe: HashSet<(Symbol, Symbol)>,
}
```

**Pipeline integration:**
```
Parse → TypeCheck → AbstractInterp(init from refinements) → Optimize → Codegen(uses ConstraintEnv)
```

AbstractInterp takes refinement predicates as initial constraints. Results persist through to codegen. Peephole queries `proven_safe` instead of doing its own syntactic analysis. RefinementContext checks `ranges` before emitting assertions — if the range proves the predicate, skip the `debug_assert!`.

---

## 4. Specialization Boundaries

### The Architecture

```
PE (partial_eval.rs)           Supercompiler (supercompile.rs)
├─ Scope: inter-function       ├─ Scope: intra-function
├─ Unit: call sites             ├─ Unit: expressions within a body
├─ Key: (func, [arg_values])   ├─ Key: (program_point, abstract_store)
├─ Creates: new function        ├─ Creates: simplified expressions
│  variants (f_s0_5)           │  within existing functions
├─ Stops at: 8 variants,       ├─ Stops at: 16 history entries,
│  depth 16, IO/Write          │  Index/Slice/Call pass-through,
│  effects, embedding          │  loop-modified vars widened
└─ Cache: SpecKey → Symbol     └─ Memo: (func, args) → Value
```

### Hard Boundaries and Why They Exist

**1. Function-level only (PE)**

The PE identifies `Expr::Call` nodes, builds a SpecKey from the arguments, and creates a clone of the entire function body with static args substituted. It cannot specialize *within* a function — no branch-specific specialization, no loop iteration specialization.

**Why:** Specialization creates new top-level function definitions. There's no mechanism to say "in this branch, inline version A; in that branch, inline version B." The AST representation doesn't support function variants scoped to branches.

**2. Literal-only SpecKey (PE)**

`SpecKey = (Symbol, Vec<Option<Literal>>)`. A static argument must be a `Literal` — integers, floats, booleans, strings, nothing. Compound static values (lists, tuples, structs) are classified as `None` (dynamic) even when all their elements are known.

**Why:** The embedding check (`literal_embeds`) only works on scalar values. There's no tree-structure embedding for compound data. FIX_SPECIALIZER_PLAN Sprint C.1.5 addresses this with "partially-static structures."

**3. Loop widening erases knowledge (Supercompiler)**

When the supercompiler enters a loop, it removes ALL modified variables from the store (line 629-631):
```rust
for sym in &modified {
    env.store.remove(sym);  // Widen to completely unknown
}
```

No MSG (Most Specific Generalization), no precision preservation. The embedding check at line 646 is **dead code** — it computes but the result is never used for widening decisions.

**4. Index/Slice/Call pass-through (Supercompiler)**

Lines 479-483: `Expr::Index` and `Expr::Slice` are explicitly not driven. Lines 462-474: `Expr::Call` is only simplified in its arguments, not inlined. This preserves patterns for the codegen peephole (swap detection, for-range, buffer reuse). The trade-off: the supercompiler can't simplify `item 5 of [1,2,3,4,5,6]` to `5`.

**5. Embedding is not a WQO (PE)**

The PE's embedding check uses `literal_embeds` which compares absolute values for numbers and symbol indices for strings. This is NOT a well-quasi-ordering (WQO) by Kruskal's theorem. Consequence: termination is not mathematically guaranteed for all inputs — it relies on the variant count limit (8) as a safety net.

### The Gap Between PE and Supercompiler

The PE works inter-function but creates new function bodies. The supercompiler works intra-function but doesn't create new functions. The gap:

1. **PE specializes a function → supercompiler doesn't re-drive the residual.** After PE creates `f_s0_5`, the specialized body goes through fold/DCE but NOT through the supercompiler's symbolic execution. Constant propagation within the specialized body relies on the fixpoint fold→propagate loop, not on driving.

2. **Supercompiler simplifies expressions → PE doesn't re-specialize.** If the supercompiler folds `f(3+2)` to `f(5)`, that's now a call with a literal arg — a PE candidate. But PE already ran.

3. **No cross-boundary inlining.** The PE substitutes static args globally but doesn't inline the function body at the call site. The supercompiler inlines pure function calls at the store level (for Let/Set) but not at the AST level. Neither pass produces fully inlined code at the AST.

### "The Trick" — What Self-Application Actually Changes

From FIX_SPECIALIZER_PLAN.md (Jones et al. 1993, Section 7.2):

The PE is written in LOGOS (`pe_source.logos`). It dispatches on CExpr variants via `Inspect`:

```logos
Inspect expr:
  When CInt(v): ...
  When CAdd(a, b): ...
  When CCall(f, args): ...
```

When the PE specializes **itself** with the interpreter as static input (`pe(pe, interpreter)`), these `Inspect` dispatches should become static — the PE knows which CExpr variant it's processing because the interpreter's source code is the static argument.

**Today this doesn't happen because:**
- The PE source checks `isLiteral` but not `isStatic` — compound static values (CExpr trees) aren't recognized
- `CInspect` in the PE source doesn't do static dispatch — it processes all arms regardless
- `CRepeat` doesn't unroll when the iterable is a static collection
- There's no decompilation from CExpr/CStmt back to LOGOS source for recursive self-application

**When The Trick works:** The PE's dispatch becomes a no-op during P2. The residual is a *compiler* — direct code emission with zero interpretive overhead (no env lookups, no funcs lookups, no CExpr dispatch). This is Jones optimality.

**What changes about boundaries:** The specialization unit stays function-level. But the *depth* of specialization increases qualitatively — the PE can now specialize the PE specializing the interpreter, eliminating two levels of dispatch rather than one. The 12 sprints in FIX_SPECIALIZER_PLAN build toward this by fixing each shortcut: Sprint C (static dispatch), Sprint C.4 (environment splitting), Sprint E (real P1), Sprint F (real P2/P3).

---

## 5. Typestate Pipeline: Encoding Pass Ordering in the Type System

### The Insight

Every pass currently has the same signature:
```rust
fn fold_stmts(stmts: Vec<Stmt>, ...) -> Vec<Stmt>
fn licm_stmts(stmts: Vec<Stmt>, ...) -> Vec<Stmt>
fn abstract_interp_stmts(stmts: Vec<Stmt>, ...) -> Vec<Stmt>
```

There's nothing stopping you from calling `licm_stmts` before `abstract_interp_stmts`, even though LICM benefits from range info. The ordering is enforced by *convention* (the sequence in `optimize_program`), not by the type system.

**The fix:** Make the type parameter carry both *proof of prior passes* and *analysis data*. Each pass wraps its output in a type that encodes what's been done and what information is now available.

### Design: Stacked Type Parameters

```rust
use std::marker::PhantomData;

/// A program at a specific point in the optimization pipeline.
/// `S` encodes what passes have run and what analysis data is available.
pub struct Program<'a, S> {
    pub stmts: Vec<Stmt<'a>>,
    pub state: S,
}

// === State markers (each wraps prior state) ===

/// Raw parsed program. No optimizations applied.
pub struct Parsed;

/// Constants have been folded (2+3 → 5, algebraic identities).
pub struct Folded<S>(pub S);

/// Values have been propagated through bindings.
pub struct Propagated<S>(pub S);

/// Functions have been specialized for known arguments.
pub struct Specialized<S> {
    pub prev: S,
    pub variant_count: HashMap<Symbol, usize>,
}

/// Compile-time function evaluation complete.
pub struct CtfeApplied<S>(pub S);

/// Common subexpressions have been eliminated.
pub struct CseApplied<S>(pub S);

/// Range analysis complete. CARRIES DATA — intervals available downstream.
pub struct RangeAnalyzed<S> {
    pub prev: S,
    pub ranges: HashMap<Symbol, Interval>,
    pub lengths: HashMap<Symbol, Interval>,
    pub proven_safe: HashSet<(Symbol, Symbol)>,  // (index_var, collection)
}

/// Loop-invariant code has been hoisted.
pub struct LicmApplied<S>(pub S);

/// Accumulator loops replaced with closed-form formulas.
pub struct ClosedFormApplied<S>(pub S);

/// Producer-consumer loop chains fused.
pub struct Deforested<S>(pub S);

/// Dead code eliminated.
pub struct DceApplied<S>(pub S);

/// Supercompilation complete.
pub struct Supercompiled<S>(pub S);
```

### How Passes Declare Their Requirements

```rust
/// Fold can run on anything — no prerequisites.
pub fn fold<'a, S>(program: Program<'a, S>, ...) -> Program<'a, Folded<S>> {
    let stmts = fold::fold_stmts(program.stmts, ...);
    Program { stmts, state: Folded(program.state) }
}

/// Propagation requires folded input.
pub fn propagate<'a, S>(program: Program<'a, Folded<S>>, ...) -> Program<'a, Propagated<Folded<S>>> {
    let stmts = propagate::propagate_stmts(program.stmts, ...);
    Program { stmts, state: Propagated(Folded(program.state.0)) }
}

/// LICM benefits from range info — type system ENFORCES this.
pub fn licm<'a, S>(program: Program<'a, RangeAnalyzed<S>>, ...) -> Program<'a, LicmApplied<RangeAnalyzed<S>>> {
    // Can ACCESS program.state.ranges here — zero-trip loop safety check
    let ranges = &program.state.ranges;
    let stmts = licm::licm_stmts_with_ranges(program.stmts, ranges, ...);
    Program { stmts, state: LicmApplied(program.state) }
}

/// Codegen needs range info for bounds elision + refinement predicates for assertions.
pub fn codegen<'a, S: HasRanges + HasRefinements>(
    program: Program<'a, S>, ...
) -> String {
    let ranges = program.state.ranges();        // from abstract-interp
    let predicates = program.state.refinements(); // from type-checking
    // Use ranges to skip debug_assert! when provably safe
    // Use proven_safe to emit get_unchecked
    codegen::generate(program.stmts, ranges, predicates, ...)
}
```

### The Key Trick: State Carries Data, Not Just Proof

This is where it gets powerful. `RangeAnalyzed<S>` doesn't just prove "abstract interpretation ran" — it **carries the interval map**. Downstream passes access it through the type:

```rust
// LICM checks: "does this loop execute at least once?"
fn is_safe_to_hoist(expr: &Expr, ranges: &HashMap<Symbol, Interval>) -> bool {
    // If loop bound is provably > 0, safe to hoist panicking expressions
    if let Some(interval) = ranges.get(&loop_bound_sym) {
        interval.lo > Bound::Finite(0)
    } else {
        false // conservative
    }
}
```

Without the typestate, this data flows through `OptContext` or global state — error-prone, no compile-time guarantee it's available. With the typestate, if you write a pass that needs ranges and forget to require `RangeAnalyzed<S>`, **the code doesn't compile**.

### Trait Bounds for Flexible Requirements

Passes that need specific analysis data use trait bounds:

```rust
/// Trait for states that carry range information
pub trait HasRanges {
    fn ranges(&self) -> &HashMap<Symbol, Interval>;
    fn lengths(&self) -> &HashMap<Symbol, Interval>;
    fn proven_safe(&self) -> &HashSet<(Symbol, Symbol)>;
}

/// Trait for states that carry refinement predicates
pub trait HasRefinements {
    fn refinements(&self) -> &HashMap<Symbol, Vec<LogicExpr>>;
}

// Implement for RangeAnalyzed and anything that wraps it
impl<S> HasRanges for RangeAnalyzed<S> {
    fn ranges(&self) -> &HashMap<Symbol, Interval> { &self.ranges }
    fn lengths(&self) -> &HashMap<Symbol, Interval> { &self.lengths }
    fn proven_safe(&self) -> &HashSet<(Symbol, Symbol)> { &self.proven_safe }
}
impl<S: HasRanges> HasRanges for LicmApplied<S> {
    fn ranges(&self) -> &HashMap<Symbol, Interval> { self.0.ranges() }
    // ...delegates to inner state
}
impl<S: HasRanges> HasRanges for Deforested<S> {
    fn ranges(&self) -> &HashMap<Symbol, Interval> { self.0.ranges() }
}
// etc. — any wrapper over a state with ranges also has ranges
```

This means: once range analysis runs, **every subsequent pass can access ranges through the type**. You never lose the data. And a pass that needs ranges but runs before abstract-interp simply **won't compile** — the trait bound `HasRanges` isn't satisfied.

### This Solves the Refinement Unification Problem

The three disconnected systems (RefinementContext, AbstractInterp, Z3) become type parameters:

```rust
/// After type-checking: carries refinement predicates
pub struct WithRefinements<S> {
    pub prev: S,
    pub predicates: HashMap<Symbol, Vec<LogicExpr>>,
}

/// After abstract interpretation: carries numeric ranges
pub struct WithRanges<S> {
    pub prev: S,
    pub ranges: HashMap<Symbol, Interval>,
    pub lengths: HashMap<Symbol, Interval>,
}

/// After Z3 verification: carries proof results
pub struct WithProofs<S> {
    pub prev: S,
    pub proven_safe: HashSet<(Symbol, Symbol)>,
}
```

Codegen requires ALL of them:
```rust
pub fn codegen<'a, S: HasRanges + HasRefinements + HasProofs>(
    program: Program<'a, S>
) -> String {
    let ranges = program.state.ranges();
    let preds = program.state.refinements();
    let proofs = program.state.proofs();
    // Use ALL three:
    // 1. Skip debug_assert! when ranges prove the predicate
    // 2. Emit get_unchecked when proofs say access is safe
    // 3. Fall back to runtime assert only when neither can prove
}
```

If you forget to run abstract interpretation, codegen **won't compile** because `HasRanges` isn't satisfied. The unification happens through the type system — no shared global `ConstraintEnv`, no manual threading of data structures.

### This Also Solves the Plugin Problem

A plugin declares its type signature:

```rust
struct MyLoopTiler;

impl OptimizationPass for MyLoopTiler {
    type Input = CseApplied<RangeAnalyzed<S>>;   // needs CSE + ranges
    type Output = Tiled<CseApplied<RangeAnalyzed<S>>>;

    fn run<'a>(&self, program: Program<'a, Self::Input>) -> Program<'a, Self::Output> {
        let ranges = program.state.ranges(); // type-safe access
        // ... tiling logic ...
    }
}
```

The pipeline builder checks at compile time (for Rust plugins) or at load time (for WASM plugins, via a manifest) that the plugin's requirements are satisfied by the point it runs.

### Fixpoint Loops With Typestate

The fixpoint loop is the tricky part. Fold→propagate→PE cycles change the type each iteration: `Folded<Parsed>` → `Propagated<Folded<Parsed>>` → `Specialized<Propagated<Folded<Parsed>>>` → `Folded<Specialized<...>>` → ...

The type grows unboundedly. Solution: **erase to a fixpoint type after convergence**:

```rust
/// After the fixpoint loop converges, collapse the type stack
pub struct FixpointConverged {
    pub iterations: usize,
    pub variant_count: HashMap<Symbol, usize>,
}

pub fn fixpoint_loop<'a>(
    program: Program<'a, Parsed>, ...
) -> Program<'a, FixpointConverged> {
    let mut stmts = program.stmts;
    let mut variant_count = HashMap::new();
    let mut iterations = 0;
    for _ in 0..8 {
        let folded = fold::fold_stmts(stmts, ...);
        let propagated = propagate::propagate_stmts(folded, ...);
        let (specialized, changes) = partial_eval::specialize_stmts_with_state(
            propagated, ..., &mut variant_count,
        );
        stmts = specialized;
        iterations += 1;
        if changes == 0 { break; }
    }
    Program { stmts, state: FixpointConverged { iterations, variant_count } }
}
```

The `FixpointConverged` type proves "fold + propagate + PE ran to convergence" without encoding each individual iteration in the type.

### What This Means for the Ordering Problem

With typestate, the pass ordering question becomes a **type error**. If deforestation creates opportunities for LICM, you encode it:

```rust
// Current (misses opportunities):
fn optimize(p: Program<Parsed>) -> Program<Supercompiled<...>> {
    let p = licm(range_analyze(cse(fold(fixpoint(p))))); // LICM before deforest
    let p = deforest(p);                                  // creates new LICM opportunities
    // Too late — LICM already ran
}

// Fixed (re-runs LICM after deforest):
fn optimize(p: Program<Parsed>) -> Program<Supercompiled<...>> {
    let p = cse(fold(fixpoint(p)));
    let p = range_analyze(p);
    let p = licm(p);              // first LICM pass
    let p = deforest(p);          // creates new opportunities
    let p = licm(p);              // second LICM pass — type system allows it
    // ...
}
```

The type system doesn't *force* you to re-run passes, but it makes it **trivially safe** to do so. And it makes it **impossible** to run a pass before its prerequisites.

---

## Summary

| Topic | Key Finding | Severity |
|---|---|---|
| **Pass ordering** | Linear chain misses deforest→LICM, closed-form→CSE, abstract-interp→fold opportunities | ~5% optimization quality lost |
| **Supercompiler gaps** | Dead embedding code, Index/Slice/Call pass-through, aggressive loop widening | Architectural — intentional trade-off for peephole preservation |
| **Plugin system** | Zero infrastructure exists; needs `trait OptimizationPass` + pipeline slots + WASM/dylib loading | Major feature — could enable community optimization passes |
| **Tiered compilation** | Not implemented; trivial with trait-based passes (just filter the pass list) | Easy win for developer experience |
| **Refinement + intervals** | Three disconnected systems; abstract-interp results discarded before codegen | Moderate — connecting them unlocks bounds elision + assertion elimination |
| **Specialization boundary** | Function-level (PE) + intra-function (supercompiler) with a gap between them | Fundamental — closing the gap requires PE→supercompiler re-driving |
| **SpecKey limitations** | Literal-only, not WQO, no partially-static structures | Critical for Futamura projections; addressed by FIX_SPECIALIZER_PLAN |
| **Self-application (The Trick)** | PE source lacks static dispatch, isStatic checks, and decompilation | 12 sprints planned to fix; blocks real P2/P3 |
| **Typestate pipeline** | Encode pass ordering + analysis data in type parameters; `RangeAnalyzed<S>` carries intervals, `HasRanges` trait bounds enforce prerequisites | Solves ordering, unification, and plugin safety in one design |

---

## Interview Log

This section documents the question-and-answer process that led to the typestate design.

### Opening Questions (User)

> Questions about our stuff. In optimizations of FUTURISTIC.md we say order doesn't matter,
> but are we leaving some optimizations on the table by them not being ordered? FUTURISTIC.md,
> then the second one, it would be kind of cool time to test and time, hot-swap and pluggable,
> we want a fast compiler, hot-reloading. For polyhedral tiling does it generalize to arbitrary
> dimensionality? For bounds constraints are they implemented with refinement types? Something
> about futuristic seemed like specialization was happening at a function, or what boundaries
> are the specialization confined to? We've done a lot of work, so tell me how it relates and
> help me answer these? How also would we ship plug-ins to people at run-time?

### Initial Analysis (7 answers)

1. **Pass ordering:** Invariant 3 says "order doesn't matter" but that's only true for *correctness*, not *quality*. The pipeline IS carefully ordered — fold before propagate, PE before CTFE, abstract-interp before DCE. We're leaving ~5% on the table because the post-fixpoint passes run exactly once in a linear chain. Opportunities created by later passes aren't fed back to earlier ones.

2. **Hot-swap/pluggable/fast compiler:** Nothing exists. The compiler is monolithic. The natural shape would be a `trait OptimizationPass` with `run()`, `depends_on()`, and `priority()`, enabling dynamic registration, per-pass timing, and tiered compilation modes.

3. **Polyhedral tiling to arbitrary N:** Yes, the polyhedral model generalizes to Z^N by definition. Camp 7 restricts to 3D pragmatically because matrix multiply is the critical case and 4D+ loop nests basically never appear in LOGOS programs.

4. **Bounds via refinement types:** Two disconnected systems exist. `RefinementContext` in codegen stores predicates and emits `debug_assert!`. `AbstractInterp` does interval analysis. They don't talk to each other.

5. **Specialization boundaries:** Function-level, polyvariant. SpecKey = (function, argument values). Gated by IO effects, depth limit (16), variant limit (8), and embedding checks. The supercompiler goes intra-function via symbolic execution.

6. **Existing work vs. FUTURISTIC.md:** Most of the mountain is already built — CSE, CTFE, LICM, closed-form, deforest, abstract-interp, PE, supercompile all exist and are wired into `optimize/mod.rs`.

7. **Shipping plugins at runtime:** WASM modules for safe distribution (sandboxed, cross-platform), dylib for power users (zero-overhead, direct AST access).

### Q: Want to dig deeper or are these answered?

**A: Dig deeper.**

### Q: Which questions do you want to dig deeper into?

Options offered: Pass ordering gaps, Plugin architecture, Refinement + intervals, Specialization depth.

**A: All four.**

### Deep Dive Phase

Four parallel investigations were launched into the actual codebase:

1. **Pass ordering** — Read `optimize/mod.rs`, `supercompile.rs`, `licm.rs`, `deforest.rs`, `abstract_interp.rs`, `closed_form.rs`, `dce.rs`. Identified concrete missed optimization chains with code line references.

2. **Refinement unification** — Read `codegen/context.rs` (RefinementContext), `optimize/abstract_interp.rs` (AbstractState), `verification/` (Z3). Found three disconnected systems that never share data.

3. **Specialization boundaries** — Read `partial_eval.rs`, `supercompile.rs`, `bta.rs`, `FIX_SPECIALIZER_PLAN.md`, `pe_source.logos`. Mapped the exact boundary between PE (inter-function) and supercompiler (intra-function).

4. **Plugin architecture** — Explored `optimize/mod.rs` pipeline structure. Confirmed zero plugin infrastructure exists.

### The Convergence

During the deep dive, the typestate insight emerged: all four problems share a root cause — **passes are untyped functions over `Vec<Stmt>`**. If the type parameter carried both proof-of-execution and analysis data:

- **Pass ordering** becomes a type error (can't call LICM before range analysis)
- **Refinement unification** becomes automatic (data flows through the type, not through global state)
- **Plugin safety** becomes a compile-time check (plugin declares its required state as a type bound)
- **Specialization boundaries** become visible in the type (PE output vs. supercompiler output are distinct types)

### Q: This typestate design touches all four questions. Continue exploring?

**A: Write a plan called TYPESTATE.md with the details and a log of our questions/interview.**
