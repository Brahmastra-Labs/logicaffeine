# EXODIA.md: The LOGOS Ultimate Compiler Architecture

## Vision Statement

To build a fundamentally new class of optimizing compiler. Rather than relying on hand-written optimization heuristics and massive backend engineering (like LLVM), LOGOS derives its compiler mechanically from its interpreter via the Third Futamura Projection. We then apply pure mathematical constraints — Abstract Interpretation, SMT Solvers, and E-Graphs — to achieve maximum theoretical execution speed with zero human-authored phase-ordering flaws.

---

## Dependency Graph

```
Tier 0: Foundation (PE soundness, Futamura Projections)
    │
    ▼
Phase 1: The Oracle (Abstract Interpretation)
    │
    ├──────────────────────┐
    ▼                      ▼
Phase 2: The Forge    Phase 4: The Architect
(SMT Superopt)        (E-Graph Equality Saturation)
    │                      │
    ▼                      │
Phase 3: Tier 1 JIT        │
(Copy-and-Patch)           │
    │                      │
    └──────────┬───────────┘
               ▼
         Phase 5: Deep Math
         (Future Horizons)
```

---

## Current Infrastructure

Before defining what we build, we document what already exists. Every phase of EXODIA extends, not replaces, this foundation.

| Component | File | Size | What Exists |
|-----------|------|------|-------------|
| Abstract Interp | `optimize/abstract_interp.rs` | 24KB | `Bound(NegInf\|Finite(i64)\|PosInf)`, `Interval{lo,hi}`, `AbstractState{vars,lengths}`, narrowing, dead-branch elimination |
| BTA | `optimize/bta.rs` | 27KB | `BindingTime(Static(Literal)\|Dynamic)`, polyvariant SCC analysis, `BtaCache`, `Division` |
| Effects | `optimize/effects.rs` | 25KB | `EffectSet{reads,writes,allocates,io,security_check,diverges,unknown}`, `EffectEnv`, purity classification |
| Partial Eval | `optimize/partial_eval.rs` | 44KB | Polyvariant specialization, embedding checks, spec keys, mixed-arg handling, BTA integration |
| PE Sources | `optimize/pe_source.logos` (59KB), `pe_bti_source.logos` (58KB), `pe_mini_source.logos` (44KB) | 161KB | Three self-referential PE implementations, all 3 Futamura projections achieved, 436 tests |
| Supercompiler | `optimize/supercompile.rs` | 36KB | Driving, folding, generalization, homeomorphic embedding, MSG, max 64 inline depth |
| GVN/CSE | `optimize/gvn.rs` | 19KB | Structural `ExprKey` hashing, common subexpression elimination |
| LICM | `optimize/licm.rs` | 15KB | Loop-invariant code motion, hoist immutable Lets above loops |
| Closed-form | `optimize/closed_form.rs` | 12KB | Gauss formula recognition for arithmetic sequences |
| Deforestation | `optimize/deforest.rs` | 19KB | Producer-consumer loop fusion |
| DCE | `optimize/dce.rs` | 14KB | Dead store and dead code elimination |
| CTFE | `optimize/ctfe.rs` | 16KB | Compile-time function evaluation |
| Fold | `optimize/fold.rs` | 26KB | Constant folding and simplification |
| Propagate | `optimize/propagate.rs` | 18KB | Value propagation via SSA-like tracking |
| Kernel E-graph | `logicaffeine_kernel/src/cc.rs` | 707 lines | `UnionFind` (path compression + union by rank), `EGraph` (hash-consing + use lists + congruence propagation), `ENode(Lit\|Var\|Name\|App)` |
| Z3 Verify | `logicaffeine_verify/src/ir.rs` | 486 lines | `VerifyExpr`, `VerifyOp(Add\|Sub\|Mul\|Div\|Eq\|Neq\|Gt\|Lt\|Gte\|Lte\|And\|Or\|Implies)`, `VerifyType(Int\|Bool\|Object)` |
| Z3 Solver | `logicaffeine_verify/src/solver.rs` | — | `Verifier`, `VerificationSession`, Z3 0.12, 10s timeout |
| Codegen | `codegen/` | 15K LOC | Rust (production), C (benchmark), FFI (WASM/Python/TypeScript) |
| AST Types | `compile.rs` | — | CExpr (34 variants), CStmt (43 variants), CVal (14 variants), CFunc, CProgram |
| Runtime | `logicaffeine_data/` | — | `LogosSeq<T>(Rc<RefCell<Vec<T>>>)`, `LogosMap<K,V>(Rc<RefCell<FxHashMap<K,V>>>)` |
| Pipeline | `optimize/mod.rs` | 93 lines | 14-pass sequential: fold → propagate → PE(x16) → CTFE → GVN → LICM → closed_form → deforest → abstract_interp → DCE → supercompile |

**Total existing optimizer:** ~300KB across 14 modules, 4513+ tests passing, 0 regressions.

---

## Tier 0: The Foundation (Semantics & Specialization)

Before we can apply advanced mathematics, the baseline partial evaluator must be perfectly sound and capable of genuine self-application.

### 0.1 Semantic Soundness

Fix the core `PEMini` bugs. Restore `collectSetVars` invalidation for dynamic `CWhile` loops and make `extractReturnM` robust against hidden early returns.

**Files:** `optimize/pe_mini_source.logos` (lines with `collectSetVarsM`, `extractReturnM`)

**Current sentinel:** `extractReturnM` uses `CVar("__no_return__")` as fallback. When a function body has CReturn inside CInspect arms, the sentinel prevents silent miscompilation. Mixed-arg CCall fallback detects sentinel and falls back to `CCall(specFunc, dynArgs)`.

**Remaining work:**
- Harden `collectSetVarsM` to invalidate staticEnv for all variables written inside dynamic CWhile bodies
- Ensure `extractReturnM` handles nested CIf/CInspect with multiple CReturn paths

### 0.2 Genuine Futamura Projections

Wire the test harness to perform actual self-application, moving away from string-replacement mocks.

**Status: COMPLETE.** All three projections achieved and verified:
- P1: `PE(interpreter, program) = compiled_program` — zero interpretive overhead (Jones optimality)
- P2: `PE(pe_source, target) = compiler_for_target` — 7 tests GREEN
- P3: `PE(pe_source, pe_bti) = compiler_generator` — 7 tests GREEN
- Genuine self-application: `PE(pe_source, pe_mini(5+3)) → "8"` — verified

**Files:** `phase_futamura.rs` (436 tests), `pe_source.logos`, `pe_bti_source.logos`, `pe_mini_source.logos`

### 0.3 The BTI (Binding-Time Interpreter)

Finalize the offline Binding-Time Analysis pass so the partial evaluator relies on explicit tags rather than online evaluation, preventing state explosion during self-application.

**Status: COMPLETE.** `bta.rs` implements polyvariant BTA with SCC-ordered analysis:
- `analyze_with_sccs()` computes `BtaCache` once, passed to `specialize_stmts_with_state()`
- Recursive functions analyzed via fixed-point iteration (max 10 per SCC)
- Non-recursive functions analyzed once
- SCCs processed in reverse topological order (Kosaraju)

### 0.4 Jones Optimality

Prove the partial evaluator completely removes interpreter dispatch overhead for isolated AST nodes, generating "pure" residual LOGOS code.

**Status: COMPLETE.** Sprint G verified: P1_real achieves zero interpretive overhead — no env/funcs lookups in residual. Tests in `phase_futamura.rs` verify no PE dispatch names, no online predicates, strict Inspect reduction.

---

## Phase 1: The Oracle (Abstract Interpretation)

**Goal:** Mathematically prove variable states ahead of time to completely eliminate runtime guards and bounds checks.

### 1.1 Define Abstract Domains

Map LOGOS types into mathematical lattices. Each domain captures a different dimension of program state.

#### Current State

`abstract_interp.rs` already implements integer interval analysis:

```rust
// EXISTS in abstract_interp.rs
enum Bound { NegInf, Finite(i64), PosInf }

struct Interval { lo: Bound, hi: Bound }

struct AbstractState {
    vars: HashMap<Symbol, Interval>,
    lengths: HashMap<Symbol, Interval>,
}
```

This is **one domain**. The Oracle requires **five**.

#### Domain 1: Intervals (upgrade existing)

Extend the existing `Interval` to support floats and add widening thresholds for loop convergence:

```rust
// UPGRADE abstract_interp.rs

enum NumericBound {
    NegInf,
    IntFinite(i64),
    FloatFinite(f64),
    PosInf,
}

struct NumericInterval {
    lo: NumericBound,
    hi: NumericBound,
}

// Widening thresholds: when a bound crosses a threshold, snap to the next
const WIDENING_THRESHOLDS: &[i64] = &[-1000, -100, -10, -1, 0, 1, 10, 100, 1000];
```

The existing `definitely_gt`, `definitely_lt`, `definitely_eq` etc. remain — they drive dead-branch elimination.

#### Domain 2: Type Domain

Track concrete types through control flow:

```rust
enum TypeTag {
    Int,
    Float,
    Bool,
    Text,
    Nothing,
    Char,
    Option(Box<TypeTag>),
    Seq(Box<TypeTag>),
    Map(Box<TypeTag>, Box<TypeTag>),
    Set(Box<TypeTag>),
    Struct(Symbol),
    Enum(Symbol),
    Closure { param_count: usize },
}

enum TypeAbstraction {
    Concrete(TypeTag),             // exactly this type
    Union(HashSet<TypeTag>),       // one of these types
    Top,                           // unknown
}
```

**Lattice:** `Concrete(T) ⊏ Union({T, U, ...}) ⊏ Top`. Join is union of type sets. Meet is intersection. When Union has one element, collapse to Concrete.

**Use:** When the Oracle proves a variable is `Concrete(Int)`, the PE can specialize a generic function to the integer-only path, eliminating type dispatch.

#### Domain 3: Collection Shape

Track collection sizes more precisely than the existing `lengths` HashMap:

```rust
enum CollectionShape {
    Empty,                         // length == 0
    Singleton,                     // length == 1
    KnownSize(usize),             // length == n
    SizeRange(usize, usize),      // length in [lo, hi]
    NonEmpty,                      // length >= 1
    Top,                           // unknown
}
```

**Lattice:** `Empty ⊏ Singleton ⊏ KnownSize(n) ⊏ SizeRange(lo, hi) ⊏ NonEmpty ⊏ Top`. Push narrows shape (Empty → Singleton, KnownSize(n) → KnownSize(n+1)). Pop widens.

**Use:** When the Oracle proves a collection is `KnownSize(4)` and an index is `Interval(0, 3)`, the bounds check is mathematically impossible to fail — the PE drops it from the residual.

#### Domain 4: Nullability

Track whether Option values are definitely Some, definitely None, or unknown:

```rust
enum Nullability {
    Definite,    // definitely Some
    Null,        // definitely None
    Maybe,       // unknown
}
```

**Lattice:** `Definite ⊏ Maybe`, `Null ⊏ Maybe`. Join: `Definite ⊔ Null = Maybe`.

**Use:** After an `Inspect x: When Some(v): ...` arm, the Oracle proves `v` is `Definite`. The PE eliminates the unwrap guard.

#### Domain 5: Alias Domain (hardest — implemented last)

Track whether two symbols may point to the same `Rc<RefCell<_>>`:

```rust
enum AliasInfo {
    Unique,                        // no other name for this allocation
    MustAlias(Symbol),             // definitely aliases this symbol
    MayAlias(HashSet<Symbol>),     // might alias any of these
}
```

**Lattice:** `Unique ⊏ MustAlias(s) ⊏ MayAlias({s, t, ...})`. A mutation through any alias invalidates abstract facts for all aliases.

**Why this is hard:** LOGOS uses `LogosSeq<T>(Rc<RefCell<Vec<T>>>)` for reference semantics. `Let a be items.` creates an alias — `a` and `items` point to the same `Rc`. `Push 1 to a.` mutates via `a`, which must invalidate facts about `items`. Without alias analysis, the Oracle must conservatively assume every mutation affects every collection.

### 1.2 The Galois Connection

The Galois connection (α, γ) between concrete domain C and abstract domain A provides mathematical soundness:

- **α: C → A** (abstraction): maps a set of concrete values to the best abstract approximation
- **γ: A → C** (concretization): maps an abstract value to the set of concrete values it represents
- **Soundness guarantee:** `∀c ∈ C: c ⊆ γ(α(c))` — the abstraction never loses concrete values
- **Optimality:** `∀a ∈ A: α(γ(a)) ⊑ a` — concretizing and re-abstracting doesn't lose precision

For the BTA pass, this means:
- If the Oracle says `x ∈ Interval(0, 255)`, then **every possible concrete execution** has `x` in `[0, 255]`
- If the Oracle says `list` has `CollectionShape::KnownSize(4)`, then **no execution** has the list at a different length at that program point

### 1.3 Guard Elimination

When the Oracle proves a guard condition, the partial evaluator drops it from the specialized output:

| Oracle Proof | Guard Eliminated |
|---|---|
| `x ∈ Interval(0, len-1)` | Bounds check on `Index(list, x)` |
| `x: TypeAbstraction::Concrete(Int)` | Type dispatch in generic function |
| `opt: Nullability::Definite` | Option unwrap check |
| `list: CollectionShape::NonEmpty` | Empty-check before `Pop` or `Index(list, 0)` |
| `a: AliasInfo::Unique` | Copy-on-write guard before mutation |

### 1.4 Product Lattice

All five domains combined into a single abstract value:

```rust
struct AbstractValue {
    interval: NumericInterval,
    ty: TypeAbstraction,
    shape: CollectionShape,
    nullability: Nullability,
    alias: AliasInfo,
}

impl AbstractValue {
    fn top() -> Self {
        AbstractValue {
            interval: NumericInterval::top(),
            ty: TypeAbstraction::Top,
            shape: CollectionShape::Top,
            nullability: Nullability::Maybe,
            alias: AliasInfo::MayAlias(HashSet::new()),
        }
    }

    fn join(&self, other: &Self) -> Self {
        AbstractValue {
            interval: self.interval.join(&other.interval),
            ty: self.ty.join(&other.ty),
            shape: self.shape.join(&other.shape),
            nullability: self.nullability.join(&other.nullability),
            alias: self.alias.join(&other.alias),
        }
    }

    fn widen(&self, other: &Self) -> Self {
        AbstractValue {
            interval: self.interval.widen(&other.interval),
            ty: self.ty.join(&other.ty),       // no widening needed for finite lattice
            shape: self.shape.widen(&other.shape),
            nullability: self.nullability.join(&other.nullability),
            alias: self.alias.join(&other.alias),
        }
    }
}

struct RichAbstractState {
    vars: HashMap<Symbol, AbstractValue>,
}
```

### 1.5 The AbstractDomain Trait

```rust
trait AbstractDomain: Clone + PartialOrd {
    fn top() -> Self;
    fn bot() -> Self;
    fn join(&self, other: &Self) -> Self;   // least upper bound
    fn meet(&self, other: &Self) -> Self;   // greatest lower bound
    fn widen(&self, other: &Self) -> Self;  // for loop convergence
}
```

Each of the five domains implements this trait. The product lattice's operations are defined componentwise.

### 1.6 Integration

**Files to modify:**
- `optimize/abstract_interp.rs` — extend from interval-only to `RichAbstractState`
- `optimize/mod.rs` — thread `RichAbstractState` from abstract_interp to PE and e-graph
- `optimize/partial_eval.rs` — use type/interval/nullability info for more aggressive specialization
- `optimize/supercompile.rs` — use alias info for precise store invalidation

**New function signature:**
```rust
// abstract_interp.rs
pub fn rich_abstract_interp_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> (Vec<Stmt<'a>>, RichAbstractState)
```

### 1.7 Sprint Roadmap

| Sprint | Goal | Test Gate |
|--------|------|-----------|
| 1 | Refactor `abstract_interp.rs` to `AbstractDomain` trait. Extract `Interval` as trait impl. | All existing tests pass. |
| 2 | Add `TypeAbstraction` domain. Track types through Let/Set/If/While. | `Let x be 5.` → Oracle knows `x: Concrete(Int)`. |
| 3 | Add `CollectionShape` domain. Track Push/Pop/New effects on sizes. | `Let xs be a new Seq of Int. Push 1 to xs.` → Oracle knows `xs: Singleton`. Dead branch `If length of xs is 0` eliminated. |
| 4 | Add `Nullability` domain. Track Option Some/None through Inspect arms. | After `Inspect x: When Some(v): ...`, Oracle knows `v: Definite`. |
| 5 | Add `AliasInfo` domain. Track Rc clone vs deep_clone. | `Let a be items. Push 1 to a.` → Oracle invalidates `items` facts. |
| 6 | Assemble product lattice. Wire `RichAbstractState` into `optimize_program`. | Combined scenarios. All 4513+ tests still pass. |

### 1.8 Testing

**New file:** `phase_exodia_oracle.rs`

Test patterns:
1. **Domain unit tests** inside `abstract_interp.rs`: join, meet, widen operations for each domain
2. **Integration tests**: compile LOGOS source, verify generated Rust code eliminates dead branches / simplifies expressions based on Oracle proofs
3. **Regression gate**: all existing tests pass with Oracle enabled

---

## Phase 2: The Forge (SMT-Backed Superoptimization)

**Goal:** Generate flawless, optimal machine code templates for the Tier 1 JIT without writing a standard compiler backend.

### 2.1 Formal Semantics Translation

Map the pure residual output of individual LOGOS AST nodes into strict logical constraints. Every CExpr variant gets a Z3 specification.

**Current Z3 infrastructure** (`logicaffeine_verify/src/ir.rs`):

```rust
// EXISTS
enum VerifyType { Int, Bool, Object }
enum VerifyOp { Add, Sub, Mul, Div, Eq, Neq, Gt, Lt, Gte, Lte, And, Or, Implies }
enum VerifyExpr { Int(i64), Bool(bool), Var(String), Binary{op,left,right}, Not(_),
                  ForAll{vars,body}, Exists{vars,body}, Apply{name,args} }
```

**Extension needed:** Add `Float`, `Seq`, `Map`, `Set`, `Struct`, `Enum` to `VerifyType`.

**Semantic specification table (core CExpr variants):**

| CExpr | Pre | Post | Z3 Encoding |
|-------|-----|------|-------------|
| `CInt(n)` | — | `result = n` | `VerifyExpr::Int(n)` |
| `CBool(b)` | — | `result = b` | `VerifyExpr::Bool(b)` |
| `CVar(x)` | `x ∈ env` | `result = env[x]` | `VerifyExpr::Var(x)` |
| `CBinOp(Add, l, r)` | `l: Int, r: Int` | `result = l + r` | `VerifyExpr::binary(Add, l, r)` |
| `CBinOp(Sub, l, r)` | `l: Int, r: Int` | `result = l - r` | `VerifyExpr::binary(Sub, l, r)` |
| `CBinOp(Mul, l, r)` | `l: Int, r: Int` | `result = l * r` | `VerifyExpr::binary(Mul, l, r)` |
| `CBinOp(Div, l, r)` | `l: Int, r: Int, r ≠ 0` | `result = l / r` | `VerifyExpr::binary(Div, l, r)` |
| `CBinOp(Mod, l, r)` | `l: Int, r: Int, r ≠ 0` | `result = l % r` | `VerifyExpr::Apply("mod", [l, r])` |
| `CBinOp(Eq, l, r)` | — | `result = (l == r)` | `VerifyExpr::binary(Eq, l, r)` |
| `CBinOp(Lt, l, r)` | `l: Int, r: Int` | `result = (l < r)` | `VerifyExpr::binary(Lt, l, r)` |
| `CNot(x)` | `x: Bool` | `result = !x` | `VerifyExpr::Not(x)` |
| `CLen(xs)` | `xs: Seq` | `result >= 0` | `VerifyExpr::Apply("len", [xs])` |
| `CIndex(xs, i)` | `xs: Seq, 0 <= i < len(xs)` | `result = xs[i]` | `VerifyExpr::Apply("index", [xs, i])` |

### 2.2 SMT Synthesis

Feed these constraints into an SMT solver (Z3 or CVC5). Ask the solver to prove the shortest possible sequence of x86_64/ARM64 instructions that satisfies the logic.

**Architecture:**

```rust
// New crate: logicaffeine_forge

struct TemplateSynthesizer {
    solver: VerificationSession,    // from logicaffeine_verify
    target: TargetArch,
}

enum TargetArch {
    X86_64,
    AArch64,
}

impl TemplateSynthesizer {
    /// Ask Z3: find the minimal instruction sequence satisfying this spec
    fn synthesize(&self, spec: &VerifyExpr, constraints: &RegisterConstraints) -> Template {
        // 1. Encode spec as Z3 assertion
        // 2. Encode target ISA subset as Z3 bitvector operations
        // 3. Ask for SAT model with minimum instruction count
        // 4. Extract machine code bytes from model
        todo!()
    }
}
```

### 2.3 The Copy-and-Patch Library

Store these superoptimized, mathematically proven machine-code snippets as immutable templates containing binary "holes" for variables and jumps.

```rust
struct Template {
    name: String,
    machine_code: Vec<u8>,          // raw bytes — the "Copy"
    relocations: Vec<Relocation>,   // the "Patch" holes
    input_types: Vec<VerifyType>,
    output_type: VerifyType,
    specification: VerifyExpr,      // Z3 proof of correctness
}

struct Relocation {
    offset: usize,                  // byte offset in machine_code
    kind: RelocKind,
}

enum RelocKind {
    AbsoluteAddress,                // patch with runtime address
    RelativeCall,                   // patch with relative call offset
    StackSlot(usize),               // patch with stack offset
    RegisterSlot(u8),               // patch ModR/M byte
}

struct TemplateLibrary {
    templates: HashMap<String, Template>,
}

impl TemplateLibrary {
    /// Serialize to binary blob for include_bytes! embedding
    fn serialize(&self) -> Vec<u8> { todo!() }

    /// Deserialize from embedded binary
    fn deserialize(bytes: &[u8]) -> Self { todo!() }
}
```

**Build-time, not runtime:** Templates are synthesized at build time via `logicaffeine_forge`, serialized, and embedded in the final binary via `include_bytes!`. Z3 is never called at runtime.

### 2.4 Sprint Roadmap

| Sprint | Goal | Test Gate |
|--------|------|-----------|
| 7 | Semantic Z3 specs for 10 core CExpr variants. Extend `VerifyType`. | Each specification is satisfiable. |
| 8 | Template synthesizer: Z3-generated instruction sequences (x86-64 only). | Synthesized `int_add` template matches specification. |
| 9 | Template library serialization to binary blobs. | Round-trip: serialize → deserialize → verify each template. |
| 10 | Copy-and-patch runtime: JIT that copies templates and patches relocations. | JIT-compile `add(a, b)`, call it, verify `add(3, 5) == 8`. |
| 11 | Extend to CStmt variants: branch (CIf), loop (CWhile), return, call. | JIT-compile factorial function, verify `factorial(10) == 3628800`. |
| 12 | Profiling-guided tier-up: interpreter counts function calls, tiers up hot functions. | Function called 1000 times in a loop gets JIT-compiled. |

### 2.5 Testing

**New file:** `phase_exodia_forge.rs`

Test patterns:
1. Z3 specification satisfiability
2. Synthesized template correctness for sample inputs
3. Serialization round-trips
4. JIT output matches interpreter output for same input

---

## Phase 3: The Tier 1 JIT (Instant Compilation)

**Goal:** Execute LOGOS code with zero latency by gluing templates together at the speed of `memcpy`.

### 3.1 Linear Scan Register Routing

Implement a lightning-fast, greedy register allocator that runs in a single pass:

```rust
// New module: codegen/jit.rs

struct RegisterAllocator {
    free_regs: Vec<Register>,      // available registers
    live_intervals: Vec<LiveInterval>,
    spill_slots: Vec<usize>,
}

struct LiveInterval {
    var: Symbol,
    start: usize,                  // instruction index
    end: usize,
    reg: Option<Register>,
}

enum Register {
    // x86-64: rax, rcx, rdx, rsi, rdi, r8-r15
    // aarch64: x0-x30
    Gpr(u8),
    // xmm0-xmm15 / v0-v31
    Fpr(u8),
}
```

Single-pass, greedy allocation: scan instructions left-to-right, expire intervals that end before current point, assign first free register or spill to stack.

### 3.2 Binary Patching

As the JIT copies SMT-generated templates into executable memory, it overwrites the ModR/M bytes to route registers and patches the jump destinations:

```rust
struct JitCompiler {
    code: Vec<u8>,                 // assembled machine code
    template_lib: TemplateLibrary, // from Phase 2
    allocator: RegisterAllocator,
}

impl JitCompiler {
    /// Compile a function to native code
    fn compile_function(&mut self, func: &CFunc) -> *const u8 {
        // 1. Allocate registers via linear scan
        // 2. For each CExpr/CStmt in body:
        //    a. Look up template in library
        //    b. memcpy template bytes into code buffer
        //    c. Patch relocations with allocated registers and jump targets
        // 3. mmap the code buffer as executable
        // 4. Return function pointer
        todo!()
    }
}
```

### 3.3 Entropic Profiling (Information Geometry)

Monitor Tier 1 execution using Shannon Entropy / KL Divergence. Only flag loops for Tier 2 optimization if the hardware branch predictor is demonstrably failing:

```rust
struct EntropyProfiler {
    branch_counts: HashMap<usize, (u64, u64)>,  // (taken, not_taken) per branch site
    threshold: f64,                               // KL divergence threshold for tier-up
}

impl EntropyProfiler {
    /// Shannon entropy of a branch site: H = -p*log2(p) - (1-p)*log2(1-p)
    fn entropy(&self, site: usize) -> f64 {
        let (taken, not_taken) = self.branch_counts[&site];
        let total = (taken + not_taken) as f64;
        let p = taken as f64 / total;
        if p == 0.0 || p == 1.0 { return 0.0; }
        -(p * p.log2() + (1.0 - p) * (1.0 - p).log2())
    }

    /// Flag for Tier 2 if entropy exceeds threshold (branch predictor is failing)
    fn should_tier_up(&self, site: usize) -> bool {
        self.entropy(site) > self.threshold
    }
}
```

**Key insight:** Low entropy (near 0.0) means the branch is predictable — the CPU's branch predictor handles it fine. High entropy (near 1.0) means the branch is unpredictable — Tier 2 should restructure the code to eliminate the branch entirely.

### 3.4 Sprint Roadmap

| Sprint | Goal | Test Gate |
|--------|------|-----------|
| 13 | Linear scan register allocator for x86-64. | Allocates registers for a 10-instruction function without spills. |
| 14 | Binary patching: memcpy templates + patch relocations + mmap executable. | JIT'd function pointer callable from Rust. |
| 15 | Entropic profiling: instrument branches, compute entropy. | Branch with 50/50 split has entropy ~1.0. Always-taken has ~0.0. |
| 16 | Tier-up wiring: interpreter → Tier 1 JIT on hot functions. | Program calling a function 10K times uses JIT'd version after threshold. |

---

## Phase 4: The Architect (Tier 2 E-Graph Optimizer)

**Goal:** Achieve global optimization for "hot" loops without the Phase-Ordering Problem.

### 4.1 Custom E-Graph Architecture

We build our own e-graph, extending the existing `UnionFind` and `EGraph` infrastructure in `logicaffeine_kernel/src/cc.rs`. No external crate dependency.

**Why custom, not `egg`:**
- Tighter integration with the LOGOS AST and the Oracle's `RichAbstractState`
- The kernel already has a production-quality `UnionFind` (path compression + union by rank) and `EGraph` (hash-consing + use lists + congruence propagation)
- Control over memory layout, node representation, and rewrite scheduling
- No external dependency for a core compiler subsystem

**Step 1: Extract `UnionFind` to `logicaffeine_base`**

The kernel's `UnionFind` (cc.rs:42-91) is generic infrastructure. Extract it to `logicaffeine_base` so both the kernel (proof terms) and the compiler (CExpr) can share it:

```rust
// logicaffeine_base/src/union_find.rs

pub type NodeId = usize;

pub struct UnionFind {
    parent: Vec<NodeId>,
    rank: Vec<usize>,
}

impl UnionFind {
    pub fn new() -> Self { ... }
    pub fn make_set(&mut self) -> NodeId { ... }
    pub fn find(&mut self, x: NodeId) -> NodeId { ... }   // path compression
    pub fn union(&mut self, x: NodeId, y: NodeId) -> bool { ... }  // union by rank
}
```

**Step 2: Define `CompilerENode`**

The kernel's `ENode` uses curried `App { func, arg }` for proof terms (De Bruijn indices). The compiler needs flat, multi-arity nodes covering CExpr:

```rust
// optimize/egraph.rs

/// Compiler-level E-Graph node
/// Flat, multi-arity — NOT curried like kernel's ENode
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompilerENode {
    // Literals
    Int(i64),
    Float(u64),       // f64 bits for Hash/Eq
    Bool(bool),
    Text(Symbol),
    Nothing,

    // Arithmetic
    Add(NodeId, NodeId),
    Sub(NodeId, NodeId),
    Mul(NodeId, NodeId),
    Div(NodeId, NodeId),
    Mod(NodeId, NodeId),

    // Shift (for strength reduction)
    Shl(NodeId, NodeId),
    Shr(NodeId, NodeId),
    BitAnd(NodeId, NodeId),

    // Comparison
    Eq(NodeId, NodeId),
    Neq(NodeId, NodeId),
    Lt(NodeId, NodeId),
    Gt(NodeId, NodeId),
    Lte(NodeId, NodeId),
    Gte(NodeId, NodeId),

    // Boolean
    And(NodeId, NodeId),
    Or(NodeId, NodeId),
    Not(NodeId),

    // Collections
    Len(NodeId),
    Index(NodeId, NodeId),
    Contains(NodeId, NodeId),
    Copy(NodeId),
    Slice(NodeId, NodeId, NodeId),

    // Structure
    FieldAccess(NodeId, Symbol),
    Call(Symbol, Vec<NodeId>),

    // Variables
    Var(Symbol),

    // Closures (for defunctionalization)
    Closure(Symbol, Vec<NodeId>),      // body_id, captured vars
    CallExpr(NodeId, Vec<NodeId>),     // closure, args
}
```

**Step 3: `CompilerEGraph`**

Extends the kernel's pattern with multi-arity congruence and per-node analysis data from the Oracle:

```rust
pub struct CompilerEGraph {
    // Core (same pattern as kernel's EGraph)
    nodes: Vec<CompilerENode>,
    uf: UnionFind,
    node_map: HashMap<CompilerENode, NodeId>,
    pending: Vec<(NodeId, NodeId)>,
    use_list: Vec<Vec<NodeId>>,

    // Analysis (Phase 1 integration)
    analysis: Vec<AbstractValue>,          // per-node Oracle data
    class_data: HashMap<NodeId, AbstractValue>,  // per-class merged analysis
}

impl CompilerEGraph {
    pub fn new() -> Self { ... }

    /// Add a node, return its canonical ID (hash-consed)
    pub fn add(&mut self, node: CompilerENode) -> NodeId { ... }

    /// Merge two nodes into the same equivalence class
    pub fn merge(&mut self, a: NodeId, b: NodeId) {
        self.pending.push((a, b));
        self.propagate();
    }

    /// Propagate congruences until fixed point (worklist algorithm)
    fn propagate(&mut self) { ... }

    /// Check multi-arity congruence (extends kernel's binary-only check)
    fn congruent(&mut self, a: NodeId, b: NodeId) -> bool {
        let na = &self.nodes[a].clone();
        let nb = &self.nodes[b].clone();
        match (na, nb) {
            (CompilerENode::Add(l1, r1), CompilerENode::Add(l2, r2)) =>
                self.uf.find(*l1) == self.uf.find(*l2) &&
                self.uf.find(*r1) == self.uf.find(*r2),
            (CompilerENode::Call(f1, args1), CompilerENode::Call(f2, args2)) =>
                f1 == f2 && args1.len() == args2.len() &&
                args1.iter().zip(args2).all(|(a, b)| self.uf.find(*a) == self.uf.find(*b)),
            // ... all node variants with children
            _ => false,
        }
    }

    /// Get the canonical representative of a node's equivalence class
    pub fn canonical(&mut self, id: NodeId) -> NodeId {
        self.uf.find(id)
    }

    /// Apply a rewrite rule to all matching nodes
    pub fn apply_rule(&mut self, rule: &RewriteRule) -> usize { ... }

    /// Run equality saturation: apply all rules until no new merges occur
    pub fn saturate(&mut self, rules: &[RewriteRule], max_iterations: usize) -> usize { ... }
}
```

### 4.2 Conversion Functions

```rust
/// Convert compiler AST expression to e-graph
pub fn expr_to_egraph(egraph: &mut CompilerEGraph, expr: &Expr<'_>) -> NodeId {
    match expr {
        Expr::Literal(Literal::Number(n)) => egraph.add(CompilerENode::Int(*n)),
        Expr::Literal(Literal::Boolean(b)) => egraph.add(CompilerENode::Bool(*b)),
        Expr::Identifier(sym) => egraph.add(CompilerENode::Var(*sym)),
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
            let l = expr_to_egraph(egraph, left);
            let r = expr_to_egraph(egraph, right);
            egraph.add(CompilerENode::Add(l, r))
        }
        // ... all Expr variants
        _ => egraph.add(CompilerENode::Var(Symbol::DUMMY)),
    }
}

/// Extract optimal expression from e-graph
pub fn egraph_to_expr<'a>(
    egraph: &mut CompilerEGraph,
    root: NodeId,
    expr_arena: &'a Arena<Expr<'a>>,
) -> &'a Expr<'a> {
    let optimal = extract_cheapest(egraph, root);
    // Convert CompilerENode back to Expr
    todo!()
}
```

### 4.3 Equality Saturation

When Tier 1 flags a megamorphic or highly complex loop, translate that subset of the LOGOS AST into an Equivalence Graph. Apply all rewrite rules simultaneously. The E-Graph doesn't overwrite nodes; it creates equivalence classes, building a massive graph of every possible valid representation.

### 4.4 Rewrite Rule Infrastructure

```rust
/// A rewrite rule: when LHS pattern matches, add RHS as equivalent
struct RewriteRule {
    name: &'static str,
    /// Try to match this rule at the given node.
    /// Returns Some(new_node_id) if the rule fires, None if it doesn't match.
    apply: fn(&mut CompilerEGraph, NodeId) -> Option<NodeId>,
}

impl RewriteRule {
    fn new(name: &'static str, apply: fn(&mut CompilerEGraph, NodeId) -> Option<NodeId>) -> Self {
        RewriteRule { name, apply }
    }
}
```

### 4.5 Rewrite Rule Catalog

#### Group 1: Algebraic Identities

```rust
// x + 0 → x
fn add_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = &eg.nodes[id].clone() {
        if let CompilerENode::Int(0) = &eg.nodes[eg.uf.find(*r)] {
            return Some(eg.uf.find(*l));
        }
        if let CompilerENode::Int(0) = &eg.nodes[eg.uf.find(*l)] {
            return Some(eg.uf.find(*r));
        }
    }
    None
}

// x * 1 → x
// x * 0 → 0
// x - 0 → x
// x - x → 0
// x / 1 → x
// !!x → x
```

#### Group 2: Boolean Simplification

```rust
// true && x → x
// false && x → false
// true || x → true
// false || x → x
// x && x → x
// x || x → x
// x && !x → false
// x || !x → true
```

#### Group 3: Strength Reduction

```rust
// x * 2 → x + x (cheaper on most architectures)
fn mul_two_to_add(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = &eg.nodes[id].clone() {
        if let CompilerENode::Int(2) = &eg.nodes[eg.uf.find(*r)] {
            let x = eg.uf.find(*l);
            return Some(eg.add(CompilerENode::Add(x, x)));
        }
    }
    None
}

// x * 2^n → x << n (shift is cheaper than multiply)
// x / 2^n → x >> n (CONDITIONAL: Oracle must prove x >= 0)
// x % 2^n → x & (2^n - 1) (CONDITIONAL: Oracle must prove x >= 0)
```

Conditional rewrites use the per-node `AbstractValue` from Phase 1:

```rust
fn div_pow2_to_shr(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Div(l, r) = &eg.nodes[id].clone() {
        if let CompilerENode::Int(n) = &eg.nodes[eg.uf.find(*r)] {
            if *n > 0 && n.is_power_of_two() {
                // CONDITIONAL: check Oracle proof that l >= 0
                let l_class = eg.uf.find(*l);
                if let Some(av) = eg.class_data.get(&l_class) {
                    if av.interval.definitely_non_negative() {
                        let shift = n.trailing_zeros() as i64;
                        let shift_node = eg.add(CompilerENode::Int(shift));
                        return Some(eg.add(CompilerENode::Shr(l_class, shift_node)));
                    }
                }
            }
        }
    }
    None
}
```

#### Group 4: Defunctionalization (Reynolds 1972)

**The problem:** If LOGOS supports functional programming (passing functions as arguments, returning functions, using `CClosure`), closures capture dynamic environment variables. Standard compilers heap-allocate them and use garbage collection. This destroys performance.

**The math:** John Reynolds mathematically proved that higher-order functions are an illusion. Every closure can be converted to a first-order tagged struct + flat dispatch.

**How it weaponizes LOGOS:**

1. Scan the AST for every `CClosure` definition. Each unique closure body gets a tag.
2. Replace `CClosure(env, body)` with `CNew("Closure_N", [captured_var1, captured_var2, ...])` — a flat struct on the stack, zero heap allocation.
3. Replace `CCallExpr(closure, args)` with `CCall("apply", [closure_struct, args...])`.
4. Generate a global `apply` function that is a flat `CInspect` (switch) on the tag:

```
Define apply taking closure and args:
  Inspect closure:
    When Closure_0(x, y):
      # body of closure 0, with x and y as captured vars
    When Closure_1(z):
      # body of closure 1, with z as captured var
```

**E-graph rewrite rules:**
```rust
// CClosure(env, body) → CNew("Closure_N", captured_vars)
fn defunctionalize_closure(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Closure(body_sym, captured) = &eg.nodes[id].clone() {
        // Generate unique tag for this closure body
        let tag = generate_closure_tag(body_sym);
        // Replace with struct construction
        return Some(eg.add(CompilerENode::Call(tag, captured.clone())));
    }
    None
}

// CCallExpr(closure, args) → CCall("apply", [closure, args...])
fn defunctionalize_call(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::CallExpr(closure, args) = &eg.nodes[id].clone() {
        let mut apply_args = vec![*closure];
        apply_args.extend_from_slice(args);
        return Some(eg.add(CompilerENode::Call(Symbol::from("apply"), apply_args)));
    }
    None
}
```

**The result:** Users write elegant, high-level functional LOGOS. The generated machine code has **zero heap allocations and zero garbage collection overhead**. It runs entirely on the stack at the speed of a raw `switch` statement.

#### Group 5: Deforestation / Fusion (Wadler 1988)

**The problem:** Modern CPUs execute instructions faster than they can fetch data from RAM. If a user writes `map(f)` then `filter(g)` then `reduce(h)`, a naive compiler allocates a new list at each step. This causes cache misses and ruins performance.

**The math:** Philip Wadler proved that the composition of recursive functions can be fused. `f ∘ g` does not need an intermediate data structure.

**How it weaponizes LOGOS:**

When the E-Graph sees a chain of collection operations, it applies fusion rules. Instead of computing list₁, storing it in RAM, and passing it to the next loop, the compiler weaves the transformations directly into one loop body.

**Existing infrastructure:** `optimize/deforest.rs` already implements producer-consumer fusion at the statement level. The E-Graph extends this to expression-level patterns.

**E-graph rewrite rules:**
```rust
// map(f, map(g, xs)) → map(f∘g, xs)
// filter(p, map(f, xs)) → filtermap(λx. if p(f(x)) then Some(f(x)) else None, xs)
// len(map(f, xs)) → len(xs)    (map doesn't change length)
// len(filter(p, xs)) → count(p, xs)  (avoid materializing filtered list)
```

**The result:** Intermediate lists never exist in memory. The CPU loads initial data into L1 cache, runs all transformations simultaneously, and writes the final result. Maximum hardware bandwidth utilization.

#### Supercompilation Integration (Turchin 1971)

**The problem:** Standard partial evaluation stops at dynamic `CWhile` loops. It leaves the loop exactly as written.

**The math:** Valentin Turchin's supervised compilation symbolically executes the multiverse of the code, building a tree of every possible state. The "whistle" (homeomorphic embedding check) detects when a branch enters a state it has seen before, and folds the infinite branch back.

**Existing infrastructure:** `optimize/supercompile.rs` already implements:
- `drive_expr` / `drive_stmt` — symbolic execution
- `embeds()` — homeomorphic embedding check
- `msg()` — most specific generalization
- `History` — bounded history (max 16 entries) for embedding checks
- Max 64 inline depth, 10K max steps

**E-graph extension:** Supercompiler results feed as equivalences into the e-graph. When the supercompiler collapses a nested state machine into a flat FSM, the FSM representation is added as an equivalent of the original loop in the e-graph. The cost extractor then picks whichever is cheaper.

### 4.6 Cost Model & Extraction

```rust
struct LogosCost;

impl LogosCost {
    /// Compute cost of a single node
    fn node_cost(node: &CompilerENode) -> f64 {
        match node {
            CompilerENode::Int(_) | CompilerENode::Bool(_) |
            CompilerENode::Nothing | CompilerENode::Text(_) => 0.0,
            CompilerENode::Var(_) => 1.0,
            CompilerENode::Add(_, _) | CompilerENode::Sub(_, _) |
            CompilerENode::Shl(_, _) | CompilerENode::Shr(_, _) |
            CompilerENode::BitAnd(_, _) => 2.0,
            CompilerENode::Mul(_, _) => 3.0,
            CompilerENode::Div(_, _) | CompilerENode::Mod(_, _) => 5.0,
            CompilerENode::Not(_) => 1.0,
            CompilerENode::And(_, _) | CompilerENode::Or(_, _) => 2.0,
            CompilerENode::Eq(_, _) | CompilerENode::Neq(_, _) |
            CompilerENode::Lt(_, _) | CompilerENode::Gt(_, _) |
            CompilerENode::Lte(_, _) | CompilerENode::Gte(_, _) => 2.0,
            CompilerENode::Len(_) => 2.0,
            CompilerENode::Index(_, _) => 4.0,
            CompilerENode::Contains(_, _) => 4.0,
            CompilerENode::Copy(_) => 8.0,
            CompilerENode::Slice(_, _, _) => 6.0,
            CompilerENode::FieldAccess(_, _) => 3.0,
            CompilerENode::Call(_, _) => 10.0,
            CompilerENode::Closure(_, _) => 15.0,    // expensive — defunctionalize!
            CompilerENode::CallExpr(_, _) => 12.0,   // expensive — defunctionalize!
        }
    }
}

/// Extract the cheapest representation from each e-class
/// Bottom-up dynamic programming: compute cost per class, pick cheapest node
fn extract_cheapest(egraph: &mut CompilerEGraph, root: NodeId) -> Vec<(NodeId, CompilerENode)> {
    let mut best_cost: HashMap<NodeId, f64> = HashMap::new();
    let mut best_node: HashMap<NodeId, NodeId> = HashMap::new();

    // Topological sort of e-classes (children before parents)
    // For each class, pick the node with minimum total cost
    // Total cost = node_cost + sum of children's best costs
    todo!()
}
```

### 4.7 Hot-Swap Architecture

When Tier 2 optimization completes, the optimized code must replace the running Tier 1 code seamlessly:

```rust
struct HotSwap {
    /// Function table: maps function name to current machine code pointer
    table: HashMap<Symbol, AtomicPtr<u8>>,
}

impl HotSwap {
    /// Atomically swap a function's code pointer
    fn swap(&self, name: Symbol, new_code: *const u8) {
        if let Some(ptr) = self.table.get(&name) {
            ptr.store(new_code as *mut u8, Ordering::Release);
        }
    }
}
```

**Tier-up flow:**
1. Tier 1 JIT runs function with entropic profiling
2. High entropy branch detected → flag for Tier 2
3. Tier 2 loads function's CExpr subtrees into `CompilerEGraph`
4. Equality saturation runs all rewrite rules (algebraic + strength reduction + defunctionalization + fusion)
5. Cost extraction picks optimal representation
6. Optimal AST lowered to machine code
7. `HotSwap::swap()` atomically replaces function pointer
8. Next call uses optimized code — no program interruption

### 4.8 Sprint Roadmap

| Sprint | Goal | Test Gate |
|--------|------|-----------|
| 17 | Extract `UnionFind` to `logicaffeine_base`. Define `CompilerENode` enum. | Kernel cc.rs uses shared `UnionFind`. `CompilerENode` covers all CExpr variants. |
| 18 | Build `CompilerEGraph`: hash-consing, multi-arity congruence propagation, worklist algorithm. | Merge x=y → Add(x,z) and Add(y,z) become equivalent (congruence). |
| 19 | Conversion: `expr_to_egraph()` and `egraph_to_expr()`. Round-trip tests. | `expr → egraph → extract → expr` preserves semantics for 50 test expressions. |
| 20 | Algebraic identity + boolean simplification rewrites (Groups 1 & 2). | `x + 0` extracts to `x`. `true && x` extracts to `x`. |
| 21 | Strength reduction + conditional rewrites via Oracle (Group 3). | `x * 4` extracts to `x << 2`. `x / 4` extracts to `x >> 2` when Oracle proves `x >= 0`. |
| 22 | Wire into `optimize_program`. Replace GVN + LICM + closed_form with e-graph pass. | All 4513+ existing tests still pass. |
| 23 | Defunctionalization (Group 4) + deforestation (Group 5) rewrite rules. | Closure-heavy program: zero heap allocs in output. Map-filter chain: one loop in output. |
| 24 | Cost extraction + hot-swap architecture. | Benchmark: e-graph pipeline faster than or equal to old sequential pipeline. |

### 4.9 Testing

**New file:** `phase_exodia_architect.rs`

Test patterns:
1. `expr_to_egraph` / `egraph_to_expr` round-trips preserve semantics
2. Each rewrite rule group produces expected simplification
3. No regressions on existing test programs
4. Cost extraction prefers cheaper representations (Shr over Div, Add over Mul*2)
5. Defunctionalization: functional programs produce zero-heap-alloc output
6. Deforestation: collection pipelines produce single-loop output

---

## Phase Integration: The Unified Pipeline

### New `optimize_program_v2`

```rust
// optimize/mod.rs

pub fn optimize_program_v2<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // ──── Stage 1: Sequential Preprocessing ────
    let bta_cache = bta::analyze_with_sccs(&stmts, interner);
    let mut current = stmts;
    let mut variant_count = HashMap::new();
    for _ in 0..16 {
        let folded = fold::fold_stmts(current, expr_arena, stmt_arena, interner);
        let propagated = propagate::propagate_stmts(folded, expr_arena, stmt_arena, interner);
        let (specialized, changes) = partial_eval::specialize_stmts_with_state(
            propagated, expr_arena, stmt_arena, interner, &mut variant_count,
            Some(&bta_cache),
        );
        current = specialized;
        if changes == 0 { break; }
    }

    // ──── Stage 2: The Oracle (Phase 1) ────
    let (range_analyzed, abstract_state) = abstract_interp::rich_abstract_interp_stmts(
        current, expr_arena, stmt_arena,
    );

    // ──── Stage 3: The Architect (Phase 4) ────
    // E-Graph replaces GVN + LICM + closed_form
    let egraph_optimized = egraph::optimize_with_egraph(
        range_analyzed, expr_arena, stmt_arena, interner, &abstract_state,
    );

    // ──── Stage 4: Cleanup ────
    let ctfe_d = ctfe::ctfe_stmts(egraph_optimized, expr_arena, stmt_arena, interner);
    let deforested = deforest::deforest_stmts(ctfe_d, expr_arena, stmt_arena, interner);
    let dce_d = dce::eliminate_dead_code(deforested, stmt_arena, expr_arena);
    supercompile::supercompile_stmts(dce_d, expr_arena, stmt_arena, interner)
}
```

### Feature Flags

```toml
[features]
default = []
rich-ai = []      # Phase 1: Rich Abstract Interpretation (Oracle)
jit = []          # Phase 2+3: SMT Superoptimization + Copy-and-Patch JIT
egraph = []       # Phase 4: E-Graph Equality Saturation (Architect)
exodia = ["rich-ai", "jit", "egraph"]  # All three pillars
```

The existing `optimize_program` remains unchanged as the default. `optimize_program_v2` is activated by the `exodia` feature flag. This guarantees zero regressions — the new passes are additive, not replacements, until they prove superior.

### Dataflow Between Phases

```
                     Oracle (Phase 1)
                     RichAbstractState
                    /                 \
                   /                   \
    Forge (Phase 2)                  Architect (Phase 4)
    Uses type info for               Uses RichAbstractState in
    correct template selection        per-node analysis for
    Uses intervals to elide           conditional rewrites
    bounds checks                     (strength reduction guards,
    Produces: TemplateLibrary         dead branch elimination)
           |                          Produces: optimized Expr
           |                                    |
    Tier 1 JIT (Phase 3)                       |
    Copy-and-Patch from                        |
    TemplateLibrary                            |
    Entropic profiling ─────────────> Tier 2 tier-up
```

---

## Phase 5: The Deep Math (Future Horizons)

These are vision statements, not implementation specifications. They represent the theoretical limits beyond EXODIA.

### 5.1 Polyhedral Tiling

Map complex nested loops into $n$-dimensional geometric polyhedra. Apply affine transformations to slice the geometry, ensuring mathematically perfect L1 cache locality and auto-vectorization.

**Connection to LOGOS:** The `optimize/closed_form.rs` pass already recognizes simple arithmetic sequences (Gauss formula). Polyhedral analysis generalizes this to arbitrary nested loop nests with affine bounds.

### 5.2 Categorical Parallelism

Represent dataflow as Symmetric Monoidal Categories. Use topological string diagrams to formally prove which execution paths can be safely multi-threaded without race conditions.

**Connection to LOGOS:** The `effects.rs` pass already tracks reads/writes/IO. Category-theoretic analysis could provide a formal foundation for the existing effect system, proving commutativity of effect-free operations.

### 5.3 Neural-Guided E-Graph Extraction

E-Graphs have one critical weakness: state explosion. If too many rewrite rules are applied, the graph consumes all available RAM. Train a machine learning model to predict the optimal path through equivalence classes, allowing thousands of rewrite rules without exploding memory.

**Connection to LOGOS:** The `LogosCost` function (Phase 4) uses hand-tuned weights. A trained model observes (e-graph structure, actual execution time) pairs and learns to predict true cost, replacing the heuristic.

### 5.4 End-to-End Formal Verification

Rewrite the compiler pipeline's core proofs in Lean or Coq to guarantee that compilation bugs are mathematically impossible.

**Connection to LOGOS:** The proof kernel in `logicaffeine_kernel` already implements the Calculus of Constructions. The `cc.rs` congruence closure tactic could be verified in Lean. Each rewrite rule in the E-Graph catalog (Phase 4) could be stated as a Lean theorem and mechanically proved sound.

---

## Design Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | **Custom e-graph, no `egg` crate** | Tighter AST integration, shared `UnionFind` with kernel, control over memory layout, no external dep for core subsystem |
| 2 | **Extract `UnionFind` to `logicaffeine_base`** | Shared between kernel (proof terms) and compiler (CExpr). Same algorithm, different node types. |
| 3 | **Flat multi-arity `CompilerENode`** | Kernel uses curried `App{func,arg}` for proof terms. Compiler needs `Add(l,r)`, `Call(f, args)`. Different representations for different domains. |
| 4 | **E-graphs for expressions only, not statements** | Statements have side effects (CShow, CPush, CWriteFile). Effect-aware term rewriting is an open research problem. Statement-level transforms remain sequential. |
| 5 | **Oracle must run before Architect** | `RichAbstractState` feeds per-node analysis in e-graph. Without Oracle proofs, conditional rewrites (strength reduction, dead branch) cannot fire. |
| 6 | **Template synthesis is offline (build-time)** | Z3 is too slow for runtime synthesis. Templates serialized to `include_bytes!` blobs. JIT at runtime is just memcpy + patch. |
| 7 | **Alias analysis is the last domain** | `Rc<RefCell<>>` reference semantics make aliasing non-trivial. Mutation through any alias invalidates all aliased facts. Build other domains first. |
| 8 | **Deforestation first among the three transformations** | Existing `deforest.rs` provides foundation. Easiest to visualize in AST. |
| 9 | **Defunctionalization most immediately powerful** | Eliminates heap allocation for closures entirely. Stack-only execution. |
| 10 | **Feature flags for gradual adoption** | `rich-ai`, `jit`, `egraph` independently toggleable. Existing pipeline unchanged as default. Zero regressions guaranteed. |

---

## Full Sprint Roadmap

### Phase 1 — The Oracle (Sprints 1-6)

| Sprint | Deliverable |
|--------|-------------|
| 1 | `AbstractDomain` trait. Refactor `Interval` to trait impl. All existing tests pass. |
| 2 | `TypeAbstraction` domain: track types through Let/Set/If/While. |
| 3 | `CollectionShape` domain: track Push/Pop/New on sizes. Dead branch elimination. |
| 4 | `Nullability` domain: track Option Some/None through Inspect. |
| 5 | `AliasInfo` domain: track Rc clone vs deep_clone. Mutation invalidation. |
| 6 | Product lattice `AbstractValue`. Wire `RichAbstractState` into pipeline. All 4513+ tests pass. |

### Phase 2 — The Forge (Sprints 7-12)

| Sprint | Deliverable |
|--------|-------------|
| 7 | Z3 semantic specs for 10 core CExpr variants. Extend `VerifyType`. |
| 8 | Template synthesizer: Z3-generated x86-64 instruction sequences. |
| 9 | Template library serialization to binary blobs. Round-trip verified. |
| 10 | Copy-and-patch runtime: JIT compiles + patches + mmap. Basic function callable. |
| 11 | CStmt coverage: If (branch), While (loop), Return, Call. Factorial works. |
| 12 | Profiling-guided tier-up: interpreter → JIT on hot functions. |

### Phase 3 — Tier 1 JIT (Sprints 13-16)

| Sprint | Deliverable |
|--------|-------------|
| 13 | Linear scan register allocator for x86-64. |
| 14 | Binary patching: memcpy + relocations + mmap executable. |
| 15 | Entropic profiling: instrument branches, compute Shannon entropy. |
| 16 | Tier-up wiring: interpreter → Tier 1 on hot functions. |

### Phase 4 — The Architect (Sprints 17-24)

| Sprint | Deliverable |
|--------|-------------|
| 17 | Extract `UnionFind` to base. Define `CompilerENode`. Kernel uses shared UF. |
| 18 | `CompilerEGraph`: hash-consing, multi-arity congruence, worklist propagation. |
| 19 | `expr_to_egraph` / `egraph_to_expr` conversion. Round-trip tests. |
| 20 | Algebraic identity + boolean simplification rewrites (Groups 1 & 2). |
| 21 | Strength reduction + conditional rewrites via Oracle (Group 3). |
| 22 | Wire into `optimize_program_v2`. Replace GVN+LICM+closed_form. All tests pass. |
| 23 | Defunctionalization (Group 4) + deforestation (Group 5) rules. |
| 24 | Cost extraction + hot-swap architecture. Benchmark vs old pipeline. |

---

## Testing Strategy

### Regression Gate

Every sprint must pass the full existing test suite:
```bash
cargo test --no-fail-fast -- --skip e2e
```
4513+ tests. Zero regressions. Non-negotiable.

### New Test Files

| File | Phase | Focus |
|------|-------|-------|
| `phase_exodia_oracle.rs` | 1 | Domain operations, guard elimination, dead branch removal |
| `phase_exodia_forge.rs` | 2 | Z3 spec satisfiability, template correctness, JIT output |
| `phase_exodia_architect.rs` | 4 | Round-trip conversion, rewrite rules, cost extraction, defunctionalization, fusion |

### Test Patterns

**Oracle:** Compile LOGOS source → verify generated Rust eliminates dead branches based on Oracle proofs.

**Forge:** Synthesize template → run on sample input → compare output to interpreter.

**Architect:** Load expression into e-graph → apply rewrites → extract → verify cheaper and semantically equivalent.

**Integration:** Run programs end-to-end with `exodia` feature flag → output matches non-optimized version.
