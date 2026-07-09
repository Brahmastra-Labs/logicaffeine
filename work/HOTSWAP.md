# HOTSWAP.md — EXODIA's Tiered, Background-Compiling, Hot-Swapping JIT (+ AOT-native tier)

> Status: spec + phased implementation plan. No code in this document changes behavior;
> it is the blueprint the phases below implement, RED-test-first.

## 1. Why

EXODIA already has two *active* optimization layers wired the wrong way around for
interactive and cold-start use — plus a third, far more powerful layer (full AOT-compiled
native code) that exists but has never been plugged into the running engine.

**Axis 1 — the AST optimizer** (`crates/logicaffeine_compile/src/optimize/mod.rs`,
`optimize_for_run`): ~16 passes, including a 16-iteration partial-evaluation fixpoint.
Today it runs **eagerly, synchronously, whole-program, before the first instruction
executes — on every run.** This is "TurboFan in the foreground": we always pay the
heaviest optimization, whether the program runs once or loops for seconds.

**Axis 2 — the native JIT** (`vm/machine.rs` region/function tiering +
`crates/logicaffeine_forge`): this layer is already adaptive. It counts per-function
calls (`hot: Vec<u32>`) and per-loop back-edges (`region_hot`), and when a unit crosses
a threshold (100) it compiles to native code and **atomically hot-swaps** it in via the
`FnTable`. Deopt has two flavors (classic replay-from-head, precise resume-at-pc). But it
**compiles synchronously on the interpreter thread** (the VM stalls during compilation),
and it is **native-only** (`forge` is `#![cfg(not(target_arch = "wasm32"))]`; the web app
does not depend on it).

**Axis 3 — the AOT compiler (latent)** (`largo build`: Logos → generated Rust → rustc/LLVM
→ machine code). This is the engine's *real* optimizing compiler — register allocation,
vectorization, inlining, `target-cpu=native` — the same output that nears C in the vs-C
benchmark, and strictly above forge's dispatch-bound copy-patch JIT. It produces a native
binary today; it has never been **plugged back into the running interpreter** as a tier.
Doing so is unique to us: our AOT target is a genuine optimizing compiler, not a faster
interpreter. The catch is latency — rustc is *seconds*, not microseconds — so Axis 3 is a
**persistent, pre-compilable, hot-swapped artifact**, not a live JIT (Axis 3 §).

Two costs follow directly:

- **The studio** (WASM, no native JIT) pays the *entire* eager Axis-1 optimizer on every
  keystroke-triggered run, for code it usually runs once. Cold-start latency with no
  payoff.
- **The vs-V8 benchmark** (`largo run --interpret`, timed per fresh process by hyperfine)
  pays the optimizer *inside the timed window*, on every run. V8, by contrast, runs
  TurboFan on a **background thread** and only on **hot** code — its optimizer cost is
  largely off the critical path. We are conservative to a fault there. (The vs-C
  benchmark builds a native binary ahead of time and times only the binary, so it is
  unaffected by run-path tiering — this whole document concerns the **run path**:
  `largo run --interpret` and the studio.)

### The goal

Make EXODIA a real TurboFan analog:

1. **Start instantly** in a baseline tier (parse → bytecode VM, no optimizer).
2. **Escalate by hotness** — cheap optimizations almost immediately, heavy ones only when
   code proves very hot.
3. **Run the heavy work in the background** and **hot-swap** the optimized code into the
   running program.
4. Keep the **same per-optimization control surface** we already use on the benchmarks
   page (`## No <X>` decorators, `OptimizationConfig` toggles), extended to govern tiering.

### Locked product decisions

- **Default = Tiered everywhere**, with **`Eager` (today's behavior) kept as a selectable
  preset**. `Eager` is the A/B baseline — "what we are at now" — that `Tiered` must beat.
  The vs-V8 benchmark runs **both** so we can watch the gap move (§12).
- **Browser: Web Workers now.** The browser's optimizing tier is re-optimized *bytecode*
  (never native — `forge` can't exist in WASM), produced in a **Web Worker** and
  hot-swapped via a bytecode side-table. This is a committed near-term phase, reusing the
  existing `WorkerOpfsVfs` worker plumbing (§9).
- **Studio cache lives in OPFS, as a sidecar to each file** (§10), via the existing `Vfs`
  API. The cached artifact is re-optimized bytecode, keyed by content hash + config.
- **AOT-native tier (Axis 3): bundle-first, opt-in, Rust-linked** (Axis 3 §). Pre-compile
  *annotated* functions into a `dlopen`'d cdylib (desktop) / pre-bundled wasm (browser) with
  a persistent hash-keyed cache; add background auto-compile later. Link the **Rust** artifact
  over the shared runtime (not the C export), with the toolchain hash in the cache key. The
  artifact is strictly optional — absent ⇒ fall through to VM+JIT, **no gaps at the seam**.

### State already in the tree (verified)

- **Tier 0 is half-built.** `interpret_for_ui_baseline*` (`ui_bridge.rs:1257`) already runs
  `with_parsed_program` (no optimizer) + `compile_with_types` (no oracle) → VM, with the
  debug shadow-oracle assert. The studio just isn't wired to it yet (still calls
  `interpret_for_ui`).
- **`OptMeta`/`REGISTRY`** (`optimization.rs:160`) has 40 rows with a `paths` bitmask but
  **no cost/tier field** — the clean extension point.
- **`FnTable`** atomic publish (`vm/native_tier.rs`) is the existing per-function hot-swap
  seam; the interpreter is its only writer.
- **Web Worker + WASM + OPFS** plumbing already exists (`logicaffeine_system::fs::WorkerOpfsVfs`,
  `crates/logicaffeine_system/src/fs/worker_opfs.rs`).

---

## 2. The tier ladder

We unify all three axes into **one five-tier ladder (T0–T4)**. The reframing: **Axis 1
stops being a whole-program pre-pass and becomes a per-unit tier transition**, **Axis 2 is
the forge representation tier**, and **Axis 3 (the AOT compiler) becomes the opt-in top
tier** (its own section below). Cost is the organizing principle — each tier spends strictly
more compile budget than the one below and is entered only when accumulated runtime
justifies it.

| Tier | Name | Default trigger | Axis 1 (AST opts applied) | Axis 2/3 (native) |
|------|------|-----------------|---------------------------|-----------------|
| **T0** | Baseline | first execution | none — raw parse → bytecode | none |
| **T1** | Warm | ~8 calls / back-edges | **Cheap** opts | — |
| **T2** | Hot | ~32 | Cheap + **Medium** (incl. PE-light) | forge JIT compiles **from T1 bytecode** |
| **T3** | Very-hot | 100 (existing native threshold) | + **Heavy** (PE-full 16-iter, unroll, recursion-unfold) | forge re-JIT **from T3 bytecode** |
| **T4** | AOT-native | opt-in: annotated + bundled/cached | full `optimize_program` (ARCHITECT) | **rustc/LLVM** cdylib (desktop) / wasm module (browser), published via `FnTable` |

**`T3` + an all-on config == today's `optimize_for_run`, bit-for-bit.** That equivalence is
the compatibility anchor (it is the `Eager` preset) and the soundness baseline (§11).

### Why three rungs, not two

The core ask is "cheap ones almost immediately, heavy ones only when very hot." Three rungs
give a smooth cost/benefit curve: cheap wins (inline, DCE) land at ~8 executions; medium
whole-function analyses (GVN, LICM, oracle, fusion, PE-light) at ~32; the expensive cloning
passes (the full 16-iteration PE fixpoint, unroll/scalarize, recursion-unfold) wait for 100
— which is exactly where the existing native-tier threshold already draws the "very hot"
line, so T3 and forge compilation align.

**T4 is not on the hotness curve.** The AOT-native tier is opt-in (annotated) and bounded by
rustc latency, so it is reached by *bundling/caching ahead of time*, not by a back-edge
counter. It is a strictly-optional top tier: if a function's AOT artifact is present it is
used from the first call; if absent, dispatch falls through to T0–T3 with no gap (Axis 3 §).

### Per-unit state machine

A "unit" is a function (keyed by `FuncIdx`) or a loop region (keyed by head pc). The
existing `NativeSlot {Untried, Failed, Ready}` and `RegionSlot {Failed, Ready}` are
generalized to a monotone tier state:

```text
Baseline{calls} --calls>=t1--> WarmPending --(bg result)--> Warm{bc}
Warm           --calls>=t2--> HotPending  --(bg result Some)--> Hot{nf}
Hot            --calls>=t3--> VeryHotPending --(bg result)--> VeryHot{nf}
any *Pending*  --(bg result None / unrepresentable)--> stay at prior tier, native side Failed
any tier       --guard miss / deopt--> stay at tier (existing demote-after-8 blacklists the region only)
```

**Escalation is monotone** — a unit never walks *down* the tier ladder (that would thrash
compile budget). The existing "demote a native region after 8 guard misses" stays as-is: it
reverts that *region* to its bytecode, which under tiering is the unit's current-tier
bytecode, still sound.

**Swap discipline (soundness):** a swap takes effect only at a **call entry** (functions)
or a **loop-head back-edge** (regions) — **never mid-iteration**. The deopt replay-from-head
contract and the array snapshot/rollback both assume entry at the head with self-consistent
registers; mid-iteration patching would break prefix-idempotence. A call/iteration in flight
finishes on whatever tier it started.

**Dispatch precedence per call:** consult the `FnTable` first (T4 AOT-native or T2/T3 forge
native — both publish there), then the Axis-1 `warm_bytecode` side-table (T1/T3 bytecode),
then the program's original bytecode (T0). Each level is one indirection; an absent
AOT-native bundle simply isn't in `FnTable`, so dispatch falls through — **no gap at the
seam**.

---

## 3. Cost model

Add a single field to the existing registry. The registry remains the one source of truth:
adding an optimization still means adding one `Opt` variant and one `OptMeta` row — now with
a cost.

```rust
/// How expensive an optimization pass is to RUN on the live path — the lever the
/// tiered optimizer uses to decide WHEN (at which hotness tier) to pay for it.
/// Orthogonal to MemClass (which is about the OUTPUT's memory cost).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OptCost { Cheap = 0, Medium = 1, Heavy = 2 }

pub struct OptMeta { /* …existing fields… */ pub cost: OptCost }

impl Opt { #[inline] pub fn cost(self) -> OptCost { self.meta().cost } }
```

The derived `Ord` makes the tier gate a single comparison: `Cheap < Medium < Heavy` is
exactly the inclusion order `T1 ⊂ T2 ⊂ T3`.

### RUN-path classification (the rows that matter)

These are the opts whose `paths` includes `RUN` — the ones `optimize_for_run` actually runs.

| Opt | RUN step | Cost | Justification |
|---|---|---|---|
| `Inline` | inline_tiny + inline_leaf | **Cheap** | single structural splice of tiny/leaf bodies, no fixpoint |
| `Tco` | self-tail → loop | **Cheap** | local return-position rewrite, one shape match |
| `DeadCode` | DCE | **Cheap** | liveness mark-and-sweep, single pass |
| `Comptime` | CTFE + refold | **Medium** | evaluate constant subtrees + one refold sweep |
| `LoopCse` | loop-carried CSE | **Medium** | one hoist pass (+ conditional refold) |
| `FloatStrength` | float induction SR | **Medium** | one rewrite pass (+ conditional refold) |
| `Cse` | GVN | **Medium** | whole-function value numbering, one pass |
| `Affine` | affine array → closed form | **Medium** | offset-array deletion + substitution |
| `LoopHoist` | LICM | **Medium** | loop-invariance analysis + motion |
| `ClosedForm` | accumulator → closed form | **Medium** | pattern-directed single pass |
| `Fuse` | deforestation | **Medium** | producer/consumer fusion, single pass |
| `Oracle` | abstract interp | **Medium** | bounded dataflow fixpoint (lattice height, not code growth) |
| `ElemType` | element-type narrowing | **Medium** | consumes Oracle facts; meaningful only warm |
| `Specialize` (PE-light) | PE fixpoint **capped at 2–4 iters** | **Medium** | a few fold→propagate→partial_eval sweeps — gives CSE/LICM/closed-form the folded/specialized code they build on, without the full-fixpoint tax |
| `Specialize` (PE-full) | **16-iter PE fixpoint** | **Heavy** | the full iterated fixpoint with polyvariant cloning — the canonical heavy pass |
| `Unroll` | constant-trip unroll | **Heavy** | grows the region |
| `Scalarize` | SROA (requires `Cse`) | **Heavy** | N scalar locals across the unrolled body |
| `Unfold` | recursion inline (last, budgeted) | **Heavy** | clones a recursive body k levels deep; keeps `MAX_RUN_INLINE_STMTS=1500` AND-ed with the tier gate |

The two special cases the design calls out:

- **Partial evaluation is split into PE-light (Medium) and PE-full (Heavy).** PE is
  *foundational* — CSE, LICM, and closed-form get most of their value from code PE has
  already folded and specialized. Classifying the whole 16-iteration fixpoint Heavy (T3-only)
  would leave T2 ("medium") with little to chew on. So a **capped 2–4-iteration PE-light**
  runs at T2 to feed the medium passes, and the **full 16-iteration PE-full** waits for T3.
  Same pass, two budgets; both semantics-preserving (§11). (Implementation: thread the
  iteration cap into the existing `for _ in 0..16` fixpoint in `optimize_for_run`.)
- **Budgeted `Unfold`** is **Heavy** and keeps its statement budget *in addition* to the
  tier gate (AND-ed): Heavy tier admits it, the budget still caps body size — the existing
  contract verbatim.

### AOT-only opts (out of scope, classified for completeness)

`Memo, Peephole, Borrow, Defunctionalize, LoopSplit, Interleave, Unbox, HoistBorrows,
Narrow, NarrowMap, DenseMap, FastDiv, OracleHints, Unchecked, Simd, Cascade, IndexString,
CapScale, Symmetry, Popcount` live in the `optimize_program` (ARCHITECT) path, not
`optimize_for_run`, so their cost is never consulted by run-path tiering. Classify the
single-sweep codegen rewrites **Cheap**, the analysis-backed ones (`OracleHints`,
`Unchecked`) **Medium**, and the unbounded-search passes (`Saturate`, `Supercompile`)
**Heavy** — for registry totality and possible future AOT reuse. `NarrowVm` (paths
`VM|RUN`) is a representation choice, not a pipeline pass; leave it governed by
`cfg.is_on` only (§4.4).

---

## 4. The tiered optimizer

### 4.1 Tier as a cost budget

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier { T0 = 0, T1 = 1, T2 = 2, T3 = 3 }

impl Tier {
    /// Highest pass cost this tier will pay. None ⇒ run nothing.
    pub fn budget(self) -> Option<OptCost> {
        match self {
            Tier::T0 => None,
            Tier::T1 => Some(OptCost::Cheap),
            Tier::T2 => Some(OptCost::Medium),
            Tier::T3 => Some(OptCost::Heavy),
        }
    }
}
```

The tier → opt-set mapping is **derived**, never hardcoded: adding an opt with a cost slots
it into the right tier automatically.

### 4.2 The gating change

State the rule once:

```rust
#[inline]
fn admits(cfg: &OptimizationConfig, tier: Tier, opt: Opt) -> bool {
    matches!(tier.budget(), Some(b) if opt.cost() <= b) && cfg.is_on(opt)
}
```

`optimize_for_run` becomes a thin wrapper so every existing caller is unchanged:

```rust
pub fn optimize_for_run<'a>(/* …existing args… */, cfg: &OptimizationConfig) -> Vec<Stmt<'a>> {
    // Back-compat: the un-tiered entry is Tier3 — today's behavior, bit-for-bit.
    optimize_for_run_tiered(/* … */, cfg, Tier::T3)
}

pub fn optimize_for_run_tiered<'a>(/* … */, cfg: &OptimizationConfig, tier: Tier) -> Vec<Stmt<'a>> {
    if cfg.is_all_off() || tier == Tier::T0 { return stmts; }
    // …existing MAX_RUN_OPT_STMTS gate unchanged…
    if admits(cfg, tier, Opt::Inline)     { /* inline_tiny + inline_leaf */ }
    if admits(cfg, tier, Opt::Specialize) { /* 16-iter PE — Heavy, T3 only */ }
    // … every step's `if cfg.is_on(X)` becomes `if admits(cfg, tier, X)`, SAME ORDER …
    if admits(cfg, tier, Opt::Unfold) && current.len() <= MAX_RUN_INLINE_STMTS { /* … */ }
    current
}
```

Invariants preserved:

- **Pass order is unchanged.** Tiering only gates *whether* a step runs, never *when*. A T2
  run is exactly the T3 pipeline with the Heavy steps elided in place.
- **`normalize`/dependency/conflict semantics are untouched** — they are properties of the
  *config*, normalized once, with the cost gate applied on top. One well-formedness rule
  makes the dependency interaction safe: **for every opt, each `requires` entry must have
  cost ≤ that opt's cost** (so a tier that admits the dependent always admits its
  dependencies). E.g. `Scalarize` (Heavy) requires `Cse` (Medium) ✓. This is a new
  registry test (§11).

---

## 5. Hotness & escalation

Reuse the existing counters — `hot: Vec<u32>` (`machine.rs:150`, bumped at `:856`) and
`region_hot: FxHashMap<usize,u32>` (`machine.rs:156`, bumped at `:236`). Replace the single
`const NATIVE_TIER_THRESHOLD = REGION_TIER_THRESHOLD = 100` with a configurable struct that
keeps 100 as the T3 (and native) threshold:

```rust
#[derive(Clone, Copy, Debug)]
pub struct TierThresholds { pub t1: u32, pub t2: u32, pub t3: u32 }
impl Default for TierThresholds { fn default() -> Self { Self { t1: 8, t2: 32, t3: 100 } } }
impl TierThresholds {
    pub fn from_env() -> Self { /* LOGOS_TIER_THRESHOLDS="8,32,100" or LOGOS_TIER_T1/T2/T3 */ }
    pub fn tier_for(self, count: u32) -> Tier {
        if count >= self.t3 { Tier::T3 } else if count >= self.t2 { Tier::T2 }
        else if count >= self.t1 { Tier::T1 } else { Tier::T0 }
    }
}
```

Defaults' rationale: don't optimize code that runs once (T1 at 8); amortize whole-function
analyses only once recurring (T2 at 32); pay the cloning passes only on genuinely hot
kernels (T3 at 100, aligned with native). `LOGOS_HOTSWAP=off` forces `tier_for` to return
T3 immediately (eager); `LOGOS_TIER_PROFILE=baseline` forces T0.

---

## 6. Background compilation (native)

Move the synchronous `tier.compile_function`/`compile_region` calls off the interpreter
thread.

```text
 interpreter thread                     compile worker thread
 ┌───────────────────┐  req (mpsc)      ┌────────────────────────┐
 │ run() dispatch     │ ───────────────>│ recv CompileRequest    │
 │  profile counters  │                 │  tier.compile_*()      │  (JitPage::new here)
 │  on threshold:     │                 │   → Box<dyn NativeFn>   │  (Send + Sync today)
 │    send request    │ <───────────────│  send CompileResult    │
 │  try_recv + publish│  res (mpsc)      └────────────────────────┘
 └───────────────────┘
```

Design points:

- **The interpreter never blocks.** It drains the result channel with non-blocking
  `try_recv` at the two existing profiling points (`try_native` `machine.rs:841`, the
  back-edge `Jump` handler `:1215`). Between request and result, the unit stays at its
  current tier (`*Pending` runs the lower tier) — exactly TurboFan's "keep running baseline
  while the optimizer works."
- **What crosses threads.** The `CompileRequest` carries owned/`'static` inputs (clone the
  function's bytecode span, an `Arc<[Constant]>` pool, `param_kinds`, `ret_kind`, the owned
  `Vec<CalleeSig>`, a `NativeCtx` clone — already `{Arc<FnTable>, Arc<AtomicI64>×2}`). The
  installed tier is already `&'static dyn NativeTier` (`OnceLock`, `Send + Sync`). The
  `Vm`, its registers, `&'p CompiledProgram`, and the thread-local `ARENA`/`ALLOC_REGISTRY`
  **never cross** — only immutable copies of compile inputs do. This avoids ever making
  `Vm: Send`.
- **The interpreter remains the single `FnTable` writer.** The worker returns the
  `Box<dyn NativeFn>`; the *interpreter* calls `FnTable.publish` (the existing atomic
  Release store). So there is no cross-thread store to the FnTable at all — the hardest
  ordering question disappears. The mpsc send/recv is the acquire/release edge that makes
  the worker-compiled `JitPage` bytes (written + sealed in `JitPage::new` on the worker)
  visible to execution on the interpreter thread. **Document this edge explicitly.**
- **Execution stays thread-pinned.** A compiled chain uses thread-local `ARENA`/
  `ALLOC_REGISTRY`, so it must *execute* on the interpreter thread. Compile off-thread,
  execute on-thread — never run a chain on the worker.
- **Run ends mid-compile is sound.** Outstanding `*Pending` results are simply never
  applied; the lower tier already produced the correct answer. At run end, drop the
  channels (the worker's `recv` errors and it exits); do **not** `join()` a mid-compile
  worker (that reintroduces blocking) — detach it. Provide a `Vm::drain_pending_compiles()`
  test hook that *does* block until idle, for deterministic differential tests (§11).
- **Gating.** Behind `LOGOS_BG_COMPILE` (default on where `std::thread` + an installed tier
  exist). The synchronous path is retained as the fallback and for the non-threaded targets.

---

## 7. Axis-1 hot-swap

> **Build coarse-first.** The headline TurboFan win is simply *moving the eager optimizer off
> the critical path* — run T0 instantly, optimize in the background, swap the optimized code
> in. The simplest form of that is **coarse**: background a single whole-program
> `optimize_for_run` and hot-swap its functions in via `FnTable` when ready (roadmap P7). The
> **per-function, cost-tiered** machinery in this section (re-parse-per-function →
> `FnBytecode` → side-table → deopt-source invariant) is a *refinement* — land it only if
> measurement shows the coarse swap leaves wins on the table. Most served benchmarks loop
> ≫100× and reach T3 under either approach, so the coarse version captures the bulk of the
> win for a fraction of the complexity.

The hard part: re-optimize a hot function's AST and swap the result into a *running*
program, given that `CompiledProgram.code` is immutable + `&'p`-borrowed and the AST arenas
are already dropped by the time the VM runs.

### Unified producer (shared front half)

```text
re-parse fn from source ─► optimize_for_run_tiered(stmts_for_fn, cfg, tier) ─► Compiler → FnBytecode
                                                                                 │
                          ┌───────────────────────────────────────────────────────┤
                   (WASM / no native)                                  (native available)
                          ▼                                                        ▼
              install in warm_bytecode[fi]                       compile_function(FnBytecode) → FnTable.publish
              (Call dispatcher indirection)                      (existing atomic swap)
```

`FnBytecode` is a self-contained, serializable per-function unit (`code: Vec<Op>` with
0-relative jump targets, its own `constants`, `register_count`, `param_count`,
`param_kinds`, `ret_kind`, `named_regs`, `loop_locals`).

**Getting the AST back:** the original arenas die when `with_optimized_program`'s closure
returns. The worker (or a WASM micro-task) **re-parses one function from source on demand**
and runs `optimize_for_run_tiered` over just that function's statements. Re-parsing one
function is cheap relative to the compile budget that justified the tier-up — far simpler
than keeping whole-program arenas alive across the run.

### The two publish paths (recommendation)

- **Primary (native): feed the re-optimized `FnBytecode` straight into `compile_function`**
  and publish to the existing `FnTable`. Maximum reuse — the atomic swap already exists and
  is proven; the inlining/specialization makes the native chain strictly better. T1 "warm"
  bytecode is produced as the *compile input* to T2/T3; on native we don't execute it, only
  compile from it.
- **WASM: the `warm_bytecode` side-table** (option a). The `Call` dispatcher and the
  dispatch loop consult `warm_bytecode[fi]` when no native entry exists; `CallFrame` carries
  the active `FnBytecode`. Pure bytecode, no `forge` — the only thing that works in the
  browser, and a genuine speedup (inline/PE/CTFE/LICM/deforestation all help the VM).

Both share the front half; they diverge only at the publish step.

**Combined on native (deopt invariant):** when a T3 native chain compiled from T1 bytecode
deopts, the replay must land on **the same bytecode the chain was compiled from** (the
`resume_pc` is relative to that body). So on native, when option (b) feeds T1 bytecode to
the JIT, **also install that bytecode in `warm_bytecode`** as the deopt-fallback source.
Invariant: *deopt always replays on the bytecode the chain was compiled from.*

### Soundness

The re-optimized bytecode is still observationally equivalent to T0 bytecode for that
function — this is exactly what `optimize_for_run` already guarantees whole-program (and it
is differentially gated in `phase_exodia_architect.rs`). Re-running it per-function reuses
that guarantee; a per-function differential (§11) catches an unsound re-optimization before
it reaches the whole-program shadow oracle. Native deopt is unchanged apart from the
"replay on the compiled-from bytecode" rule above.

---

## Axis 3 — the AOT-native tier (T4)

The most powerful and most distinctive tier: hot-swap the engine's **fully AOT-compiled
native code** (Logos → generated Rust → rustc/LLVM → machine code) into the running
interpreter. Forge (Axis 2) is a copy-and-patch JIT — fast to produce, dispatch-bound, one
stencil per op. Axis 3 is the *real* optimizing compiler: register allocation,
vectorization, inlining, `target-cpu=native` — the same output that nears C in the vs-C
benchmark. So a hot function running in the *interpreter* can hit *compiled-binary* speed.
No other interpreter has this, because their AOT target isn't a separate optimizing
compiler; ours is (rustc/LLVM).

**The reframe: it's a persistent artifact, not a live JIT.** rustc is *seconds*, not
microseconds, with no caching today. So T4 is **pre-compiled and cached**, reused across
runs — not reached by a back-edge counter. Delivery is **bundle-first** (§ below), with
background auto-compile as a later, native-only add for long-lived processes.

### It's just another `NativeTier`

The VM dispatches native code through `FnTable` (atomic entry-pointer slots); it does not
care whether a `NativeFn` came from forge stencils or a `dlopen`'d rustc artifact. Axis 3 is
an **`AotTier`** that produces `NativeFn`s from loaded artifacts and publishes them into the
*same* `FnTable`. No new dispatch path — the hot-swap seam already exists, and the
fall-through (absent artifact ⇒ no `FnTable` entry ⇒ lower tier) is automatic.

### Rust-native linkage — NOT the C export

The existing `codegen_c_export_with_marshaling` (`codegen/marshal.rs`) routes through
`logos_handle_t` + `CString` + status codes — a heavyweight C boundary. **Axis 3 links
Rust-to-Rust instead.** The interpreter (`largo`) and the bundle share the *same*
`logicaffeine_data` runtime built by the *same* toolchain, so our actual values cross by
pointer through a thin `repr(C)` *calling-convention* shim — no C marshaling. The reps
already align: `LogosSeq<T> = Rc<RefCell<Vec<T>>>` ≈ the VM's `ListRepr::Ints(Vec<i64>)`,
`LogosMap<K,V> = Rc<RefCell<FxHashMap<K,V>>>` ≈ the VM map, scalars as `i64` — the exact
boundary `ParamKind`/`PinElem` already marshals. Add a new
**`codegen_native_tier_export`** (sibling to the C export) that emits this slim shim.

Rust has no stable cross-version ABI, so the **cache key stamps the toolchain + runtime
hash**; an artifact built by a different rustc/runtime is *rejected* and dispatch falls
through to VM+JIT. Soundness over convenience — never call a mismatched artifact.

### Per-function codegen slice

Codegen is whole-program today (`codegen_program`). Add a per-function slice: extract one
`FunctionDef` (+ its transitive callees + needed `user_types`), run `optimize_program` (the
ARCHITECT path — e-graph saturate, supercompile, symmetry, loop-split) over the slice, emit
a minimal crate exposing it via the Rust-native shim, depend on `logicaffeine_data`, build to
`cdylib` (desktop) / wasm (browser). Reuses the existing standalone-`fn` emission and the
`--lib` cdylib build path (`apps/logicaffeine_cli/src/project/build.rs`).

### Delivery — bundle-first

1. **Bundle (primary).** `largo build --native-functions` pre-compiles the annotated
   functions into a loadable artifact + manifest, shipped/cached alongside the program. No
   rustc on the hot path; deterministic. This is the "tools to bundle them."
2. **Persistent cache.** Keyed by `(fn-hash, optimize_program-config, toolchain+runtime
   hash)` so a compile is reused across every run, and stale/mismatched entries are rejected.
3. **Background auto-compile (later, native only).** For a function past T3 that's still
   dominating wall time, spawn rustc in the background, cache the artifact, publish via
   `FnTable`. Pays only for long-lived/repeated runs — rustc latency makes it useless for
   short ones.

### Selectivity + graceful fall-through

Explicit annotation marks a function native — reuse the existing `is_exported`/`export_target`
mechanism (e.g. `## Native <fn>` or `## Tier <fn> native`). Marked functions are bundled
ahead. **The artifact is strictly optional:** dispatch tries `FnTable` (AOT-native or forge)
→ `warm_bytecode` → baseline; an absent or stale bundle simply isn't in `FnTable`, so the
function runs on VM+JIT with identical output. No correctness dependency on the bundle — **no
gaps at the seam.** Auto-promotion of hottest signature-clean functions is a later add.

### Loading

- **Desktop:** add `libloading` to `dlopen` the cdylib, resolve the exported symbol,
  `transmute` to the shim fn-pointer, wrap as a `NativeFn`, publish to `FnTable`. (Greenfield
  — no dynamic loading exists today, but the C-ABI export scaffolding and `cdylib` build do.)
- **Browser:** a browser **cannot** load a native `.so`/`.dylib` — only WASM runs. The
  desktop cdylib becomes a **pre-bundled wasm module** (`largo build --target wasm`),
  instantiated at runtime via the standard `WebAssembly.instantiate` and hot-swapped via the
  `warm_bytecode` side-table indirection (the entry calls into the wasm module instead of
  bytecode).

### Browser reality (honest)

The wasm module is still the full `-O3` LLVM build, so in the browser it is the **top tier**
and a large win over the bytecode VM (forge doesn't exist there at all). But it is
**near-native, not native**: the browser's own wasm engine recompiles it with no
`target-cpu=native`, mandatory bounds-checks, and indirect-call overhead — typically ~1.1–2×
off full-native depending on workload (worst on SIMD/float-heavy). Pre-bundled only; no
in-browser or server rustc for now.

### Marshaling & soundness

Scalar and `Seq<scalar>` signatures cross today via `ParamKind`/`PinElem`; Map/Text/struct
cross by pointer over the shared runtime (the types align). The entry guard checks the
signature and **bails to the lower tier on any mismatch** — AOT-native is all-or-nothing per
call, so there is **no mid-function deopt**. T4 is the easiest tier to trust: its output ==
the `largo build` binary's output, already differentially verified against expected outputs
and C. `tier_invariance` (§11) extends to T4 where a bundle exists.

---

## 8. Config surface

Keep `OptimizationConfig` as the pure "which opts are *allowed*" bitset (it is serialized,
merged per-function, normalized — don't pollute it with tier state). Add a **sibling**
`HotswapConfig` answering "*when* does each allowed opt run":

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HotswapConfig {
    pub mode: TierMode,          // Eager | Tiered | Baseline
    pub thresholds: TierThresholds,
    pins: /* per-opt tier pins; 0 = "use cost" */,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierMode { Tiered, Eager, Baseline }
```

### Presets (analogous to Speed/Memory/Safety)

| Preset | `TierMode` | Behavior |
|---|---|---|
| **Eager** | `Eager` | optimize everything upfront at T3 — **today's behavior, bit-for-bit**. The compatibility escape hatch and the **A/B benchmark baseline**. |
| **Tiered** | `Tiered` | the **new default** — units climb T0→T3 by hotness. |
| **Baseline** | `Baseline` | never optimize — every unit pinned at T0. Distinct from `LOGOS_OPT=off`: the *config* stays full, so a later `Eager`/`Tiered` run is meaningful, and it's the honest "what does raw residual cost." |

### Env vars (consistent with `LOGOS_OPT_*`)

| Var | Effect |
|---|---|
| `LOGOS_HOTSWAP=off` | force `Eager` (kill tiering, optimize upfront) — the headline escape hatch |
| `LOGOS_TIER_PROFILE=tiered\|eager\|baseline` | pick the preset (mirrors `LOGOS_OPT_PROFILE`) |
| `LOGOS_TIER_THRESHOLDS="8,32,100"` / `LOGOS_TIER_T1\|T2\|T3` | override thresholds |
| `LOGOS_TIER_PIN="specialize:eager,fuse:t1,unfold:never"` | per-opt tier pins (same keyword list grammar as `LOGOS_OPT_OFF`) |
| `LOGOS_TIER_TRACE=1` | observability (§11) |

### The decorator story (force an opt to a tier — in code)

Reuse the `## No <X>` grammar with a parallel verb so the surface is identical:

```text
## Tier <keyword> <eager|t1|t2|t3|never>
```

- `## Tier specialize eager` — run PE upfront for this program, ignore its cost.
- `## Tier fuse t1` — promote deforestation to fire as soon as warm.
- `## Tier unfold never` — allowed (`cfg.is_on`) but pinned off the tiering ladder
  (distinct from `## No unfold`, which forbids it and cascades through `normalize`).

Parsing reuses `by_keyword(word)` to resolve the opt plus a tier token. Add
`decorate_tiers(src, &[(keyword, tier)])` beside `decorate_source`. The pin overrides the
cost-derived tier:

```rust
fn admits_pinned(cfg: &OptimizationConfig, hs: &HotswapConfig, tier: Tier, opt: Opt) -> bool {
    if !cfg.is_on(opt) { return false; }
    match hs.pin(opt) {
        Some(Pin::Never) => false,
        Some(Pin::Eager) => true,            // run at any tier ≥ T1
        Some(Pin::At(t)) => tier >= t,       // run once this tier is reached
        None             => matches!(tier.budget(), Some(b) if opt.cost() <= b),
    }
}
```

---

## 9. Browser story (Web Workers now)

Constraints: no `std::thread`, `forge` compiled out, single-threaded main loop. The browser
ceiling is therefore **warm bytecode** (never native). Staged:

- **W0 — baseline (ship first).** Wire the studio to `interpret_for_ui_baseline`
  (`ui_bridge.rs:1257`, already built + differentially proven). Kills cold-start optimizer
  latency immediately. A wiring change, not new mechanism.
- **W1 — cooperative warm tier.** Add the `warm_bytecode` side-table (§7 option a) and drive
  re-optimization **cooperatively on the main thread** via `wasm-bindgen-futures`: when a
  function crosses the warm threshold, the already-`async` UI run path yields, the optimizer
  runs one function's pipeline in a micro-task, installs the `FnBytecode`, and resumes.
  Bounded work per function keeps the UI responsive.
- **W2 — Web Worker background tier (committed).** Move the optimizer pipeline into a Web
  Worker (separate wasm instance), reusing the existing `WorkerOpfsVfs` worker plumbing
  (`worker_opfs.rs`). The main-thread VM posts `{ fn_source, cfg, tier }`; the worker
  returns serialized `FnBytecode`; the main thread installs it in `warm_bytecode` and
  hot-swaps at the next call boundary. This mirrors the native worker design exactly, with
  `postMessage` replacing the mpsc channel and serialized `FnBytecode` replacing
  `Box<dyn NativeFn>`. **Native code never exists in the browser** — the worker produces
  bytecode, not machine code.

---

## 10. OPFS tier cache

The studio re-runs on every edit; identical re-runs (the "run again" case) should start
warm. Persist a **program-keyed cache of `FnBytecode`** — representation-independent and the
expensive part (the optimizer pipeline, not native emit; native chains aren't cached because
they bake per-`Vm` `FnTable` addresses).

- **Where:** in OPFS, as a **sidecar to the file**, via the existing `Vfs` trait
  (`read`/`write`/`exists`) — e.g. `/<dir>/.logoscache/<key>.bc` alongside the source. Files
  already live under `/workspace`; the studio's `WebVfs` (`get_platform_vfs_with_fallback`)
  is the handle.
- **Key:** `ProgramKey = (blake3(source_bytes), OptimizationConfig::bits())` — captures
  everything that affects compiled output (source + which opts/decorators are active).
- **Invalidation:** whole-entry on any key mismatch. Per-function granularity is unsafe
  across edits because `optimize_for_run` is whole-program (a function's optimized form can
  depend on others via inlining/PE). Identical re-runs hit wholesale; any edit recomputes —
  the correctness floor.

---

## 11. Soundness & observability

**Every opt is semantics-preserving**, so tiering must change **speed only, never output.**
The design makes that a *tested* guarantee:

- **Tiering elides whole passes in place** (same order), and each pass is individually
  output-preserving → any in-order subset is output-preserving. Different residual
  *programs* at T1 vs T3, identical *results*.
- **Monotone escalation is safe.** For strict bit-identity of the final residual, re-run
  tiered optimization **from the original AST** at the new tier (not incrementally from the
  lower-tier residual) — avoids ordering artifacts.
- **Dependency well-formedness** (`cost(requires) ≤ cost(opt)`) guarantees no opt ever runs
  in a half-applied context.

**Precondition — bit-exactness is verified, not assumed.** `tier_invariance` below asserts
*byte-identical* output across tiers, which only holds if every RUN-path opt is
**bit**-preserving, not merely value-preserving. Float passes (FloatStrength induction SR,
LICM hoisting a float computation, closed-form) reorder arithmetic; a value- but not
bit-preserving pass would fail the gate on float-heavy benchmarks. The codebase *intends*
bit-exactness ("value-preserving, no FMA", "30/30 bit-identical" in `optimize/mod.rs`), so
this should hold — but **audit every RUN-path opt for bit-exactness before P4**. If any opt
is value- but not bit-preserving, either fix it or `tier_invariance` must use ULP-tolerant
float comparison (a weaker, messier gate). This is the one assumption that could surprise us.

Tests (the load-bearing gates):

- **`tier_invariance`** — for each benchmark program (and a fuzzed corpus), run through the
  tiered path at **every** tier T0–T3, **and T4 where an AOT bundle exists**, asserting
  program output is byte-identical across all of them. The honesty contract: "tiering changes
  speed, not results." 30 benchmarks × up to 5 tiers, all bit-identical. (T4 is the easiest
  to trust: AOT-native output == the `largo build` binary's output, already differentially
  verified against expected outputs and C.)
- **`eager_equals_today`** — `optimize_for_run_tiered(.., T3)` residual == today's
  `optimize_for_run` residual over the corpus. Pins the `Eager`-is-legacy claim.
- **`requires_cost_monotone`** — every `REGISTRY` row's `requires` have cost ≤ the row's
  cost (same style as the existing `requires_and_conflicts_reference_real_variants_no_cycle`).
- **`baseline_equals_opt_off`** — T0/Baseline output == `LOGOS_OPT=off` output.
- **Force-tier mode** — `LOGOS_FORCE_TIER=warm|hot|veryhot` drops thresholds to 1 and
  `Vm::drain_pending_compiles()` makes the worker synchronous-for-tests, so the existing
  debug shadow-oracle (VM vs tree-walker, `ui_bridge.rs`) becomes a per-tier differential
  for the whole corpus, for free.
- **`tiered_reaches_t3_on_benchmark_corpus`** — under `LOGOS_TIER_TRACE`, assert every
  served benchmark climbs its hot unit(s) to T3 within its run (§12.3). This is the
  *performance* guard: it catches silent under-optimization (a benchmark stuck at T1/T2)
  that timing alone would hide. A program that genuinely can't reach T3 in its run is short
  by construction — it belongs to the cold-start metric, not the steady-state geomean.

Observability:

- **`LOGOS_TIER_TRACE=1`** (mirrors `LOGOS_JIT_TRACE`/`LOGOS_NARROW_TRACE`) emits one line
  per transition and per re-optimization, listing which opts the new tier newly admits
  (`+cse +licm +oracle …`), computed by diffing `admits` over `REGISTRY` — no separate
  bookkeeping. Surfaces per-function tier/calls/deopt counts for the studio UI.
- **`optimizations_used_by_tier(source)`** extends `optimizations_used` (`compile.rs:294`)
  to report, per tier, which opts fire — via a `compile_to_rust_at_tier` threading a `Tier`
  into `optimize_for_run_tiered`.

---

## 12. Benchmark integration

Add a **Tiered-vs-Eager axis** to `benchmarks/run-interp-vs-js.sh`: run `largo run
--interpret` under both `LOGOS_TIER_PROFILE=eager` and `LOGOS_TIER_PROFILE=tiered`,
emitting **two LOGOS rows** (plus Node) so the report (and the served benchmarks page,
which bakes `latest-interp.json`) shows the current number (`eager`) and the tiered number
side by side. This directly answers "use one as the baseline and watch the gap move." (The
vs-C track is unaffected — it times a prebuilt native binary, no run-path optimizer.)

### 12.1 Predicted effect on the served ~1.09× number (honest)

Decompose one timed `largo run --interpret` (hyperfine times the whole fresh process):

```text
T_measured = T_startup + T_parse + T_optimize + T_compile + T_execute
```

Today (**Eager**), `T_optimize` (the 16-iteration PE fixpoint et al.) is paid **upfront, on
the critical path, in every one of the 10 timed runs**. Under **Tiered**, `T_optimize` moves
**off** the critical path: the program starts executing at T0 immediately, and re-optimization
happens in the **background** (native worker, §6) and is hot-swapped in. So the change is:

- **Remove** `T_optimize` from the window (the repeated in-window tax we identified).
- **Add** a tiny warm-up: the first ~`t3` (=100) calls/back-edges run less-optimized before
  the unit reaches T3.
- **Add** minor background-compile CPU contention (a second thread during tier-up bursts).

Effect splits by benchmark character:

| Character | Reaches T3? | Net vs Eager | Why |
|---|---|---|---|
| **Long compute loops** (nbody, mandelbrot, sorts, fib, …) — most of the calibrated set | **Yes** (a tight loop does ≫100 back-edges inside its 250 ms) | **≈ Eager or slightly faster** | identical steady state (both at T3), but `T_optimize` reclaimed; warm-up is <0.01% of millions of iterations |
| **Short / cheap** (where `T_optimize` is a big fraction of the window) | maybe not | **faster** ("quick quick") | `T_optimize` skipped entirely; little execution to deoptimize |
| **Medium, never reaches T3** (rare here — calibration forces ~250 ms) | no | **risk: could be slower** | runs under-optimized the whole time; only mitigation is the warm thresholds |

So your intuition is right and the mechanism is precise: **tiering reclaims the in-window
optimizer tax, which lowers our time floor — most on the cheap/short end, least on the long
steady-state loops** (where both engines already converge to the same optimized code). Order
of magnitude: if `T_optimize` is ~2–12% of a 250 ms window on these small programs (the PE
fixpoint on a 15–30-line program is single-to-low-double-digit ms), the geomean moves from
**~1.09× toward ~1.0–1.07×** — i.e. toward / past V8 parity — *purely* from not paying the
optimizer on the clock. We have **not measured `T_optimize` yet**; it is directly
measurable as `(Eager run time − Baseline/T0 run time)` at a fixed small N, and that
measurement should be the first thing P13 produces.

The **bigger, cleaner** win shows in the **cold-start floor** (`measure-startup.sh`): a
trivial program under Tiered pays *zero* optimizer, so the serverless/CLI floor drops
sharply. Report it as two numbers — steady-state geomean **and** cold-start floor — exactly
the "two numbers" honesty the rest of the engine work uses.

### 12.2 Safe rollout — we never regress the published number

`Eager` stays a first-class preset and the A/B runs both, so the published `latest-interp.json`
can keep `eager` as the headline until `tiered` provably matches or beats it on the geomean.
**Flip the default to Tiered only when `tiered_geomean ≤ eager_geomean`** on a quiet box.
This makes the whole feature a strict ratchet: it can only improve the served number.

### 12.3 The reaches-T3 discipline (don't silently under-optimize)

The one way tiering could *quietly* cost us is a benchmark that never climbs to T3 and so
runs under-optimized. Guard it structurally, not just by timing: under
`LOGOS_TIER_TRACE`, assert every benchmark in the corpus reaches **T3** for its hot
unit(s) within its run (§13, folded into the cross-cutting gate). A benchmark that
legitimately *can't* reach T3 in its run is short by definition — its honest home is the
cold-start metric, not the steady-state geomean. Two-core CI is the contention risk: gate
the background worker to spawn only when `available_parallelism() > 2`, else fall back to
the synchronous-deferred path, so CI never has the worker stealing the interpreter's core.

### 12.4 Three bars per benchmark + the "bundle native" toggle

Per benchmark, show **three bars vs the reference (Node/V8)**: (1) **Eager VM** (today's
baseline), (2) **Tiered VM+JIT** (the TurboFan analog), (3) **AOT-native** (their compiled
native on desktop / wasm in the browser, linked in) — alongside the existing vs-C
native-vs-C track. Add a **"bundle native" toggle** on the benchmarks page, analogous to the
existing codegen optimizer toggles (the `REGISTRY` checkbox grid in `benchmarks.rs`): when
on, link the AOT-native bundle and render its bar. The page becomes a **live, interactive
demonstration of the whole tier ladder** — baseline → tiered JIT → AOT-native, per
benchmark, toggleable. `run-interp-vs-js.sh` emits the AOT-native row alongside
Eager/Tiered; the browser page shows the wasm-tier bar (near-native), the desktop report the
cdylib-tier bar (full native).

---

## 13. Phased TDD roadmap

Each phase is independently shippable, RED-test-first, smallest-valuable-first. The order is
**coarse-win-first**: the cheap policy work and the background-optimize that moves the
optimizer off the critical path land *before* the per-function Axis-1 refinements; the
AOT-native tier (Axis 3) is its own group on top.

| Phase | Change | RED test | Files |
|---|---|---|---|
| _**Group A — tiering policy** (pure, no threading)_ | | | |
| **P0** | Studio → baseline wiring | trace shows `vm (baseline)`; output matches optimized | `studio.rs` |
| **P1** | `OptCost` + classify 40 rows | `every_opt_has_cost_and_requires_are_cost_monotone` | `optimization.rs` |
| **P2** | `Tier`/`budget`/`admits` | budget + admits truth-table | `optimization.rs` |
| **P3** | `optimize_for_run_tiered` + wrapper + **PE-light/full split** | `eager_equals_today`, `tier0_is_noop` | `optimize/mod.rs` |
| **P4** | `tier_invariance` + **bit-exactness audit** | byte-identical T0–T3 | `tests/tier_invariance.rs` |
| **P5** | `HotswapConfig` + presets + env | `from_env`, `hotswap_off_forces_eager` | `optimization.rs` |
| **P6** | `## Tier` decorators + pins | `decorate_tiers` + parse `(Opt, Pin)` | `optimization.rs`, parser |
| _**Group B — the core background win**_ | | | |
| **P7** | **Coarse** background optimize + whole-program swap | bg-optimized == eager; run-ends-midflight sound | `machine.rs`, `ui_bridge.rs` |
| **P8** | Background forge compile off-thread (Axis-2) | bg == sync; off-thread page executes | `machine.rs`, `native_tier.rs` |
| _**Group C — Axis-1 refinements** (gated on measurement)_ | | | |
| **P9** | Per-function `FnBytecode` producer | `warm_bytecode_equivalent_to_baseline` | `optimize/mod.rs`, `ui_bridge.rs` |
| **P10** | Axis-1 native hot-swap | `axis1_native_matches_baseline`; deopt on T1 bytecode | `machine.rs`, `native_tier.rs` |
| **P11** | Axis-1 bytecode side-table (WASM-portable) | warm tier serves calls, no native; == baseline | `machine.rs` |
| **P12** | OPFS tier cache | hit on re-run; invalidate on edit | cache, `apps/logicaffeine_web` |
| **P13** | Web Worker browser warm tier + `LOGOS_TIER_TRACE` | wasm warm == baseline; trace format | `apps/logicaffeine_web`, `ui_bridge.rs` |
| _**Group D — Axis 3: AOT-native tier (T4)**_ | | | |
| **P14** | Rust-native export shim + per-function codegen slice | slice compiles; symbol callable; == `largo build` | `codegen/{program,marshal,ffi}.rs` |
| **P15** | `AotTier`: `libloading` load + `NativeFn` + `FnTable` publish + fall-through | dlopen'd fn == baseline; **absent bundle falls through identical** | `native_tier.rs`, `machine.rs` |
| **P16** | Bundle + persistent cache + `largo build --native-functions` | cache hit by hash; toolchain-mismatch → fall through | `project/build.rs` |
| **P17** | Browser pre-bundled wasm (`WebAssembly.instantiate` + swap) | wasm-module tier == baseline | `apps/logicaffeine_web` |
| **P18** | Background AOT auto-compile (native, later) | bg AOT cached, output matches | `machine.rs` |
| _**Group E — benchmark + UI**_ | | | |
| **P19** | 3-bar (Eager/Tiered/AOT-native) + "bundle native" toggle + AOT row | page/report shows three bars; toggle links native | `benchmarks.rs`, `run-interp-vs-js.sh` |

**Cross-cutting gate (lands with P4, tightened each phase):** `assert_all_tiers_match` over
the benchmark + test corpus, run in CI with `LOGOS_FORCE_TIER` at each level and
`drain_pending_compiles()` for determinism. No phase merges unless every tier (T0–T4, T4
where a bundle exists) produces byte-identical output to baseline. Run the full suite with
`./scripts/run-all-tests-fast.sh`. From P8 on, also run `tiered_reaches_t3_on_benchmark_corpus`
(§11) so the performance guard travels with the soundness guard; from P15 on, the
fall-through gate (absent AOT bundle ⇒ identical output via VM+JIT) travels too.

**Rollout ratchet:** the default flips to `Tiered` (P19) only once the A/B shows
`tiered_geomean ≤ eager_geomean` on a quiet box (§12.2). `Eager` stays the published headline
until then, so the served ~1.09× number can only improve, never regress. The AOT-native tier
(T4) is always opt-in, so it never affects a program that doesn't request it.

---

## 14. Key reuse points (don't reinvent)

- `OptimizationConfig` / `REGISTRY` / `Opt` / `by_keyword` / `decorate_source` / `Profile`
  (`crates/logicaffeine_language/src/optimization.rs`) — extend, don't fork.
- `FnTable` atomic publish + `NativeTier`/`NativeCtx` + two-flavor deopt
  (`crates/logicaffeine_compile/src/vm/native_tier.rs`, `vm/machine.rs`) — the hot-swap seam.
- `interpret_for_ui_baseline*` (`crates/logicaffeine_compile/src/ui_bridge.rs:1257`) — T0,
  already built.
- `optimizations_used` (`crates/logicaffeine_compile/src/compile.rs:294`) + the benchmarks
  checkbox grid (`apps/logicaffeine_web/src/ui/pages/benchmarks.rs`, ~2257) — extend with a
  cost badge per row and an Eager/Tiered/Baseline selector, injecting `## Tier` lines into
  the live decorated source exactly as it injects `## No <X>` today.
- `Vfs` / `OpfsVfs` / `WorkerOpfsVfs` (`crates/logicaffeine_system/src/fs/`) — OPFS cache +
  Web Worker plumbing.
- `codegen_c_export_with_marshaling` / `CAbiClass` / `has_wasm_exports` / `is_exported` /
  `export_target` (`codegen/{marshal,ffi}.rs`) + the `--lib` cdylib build
  (`apps/logicaffeine_cli/src/project/build.rs`) — the export + build scaffolding the
  Rust-native shim and `AotTier` extend (add `libloading` for desktop loading;
  `WebAssembly.instantiate` for the browser). The per-function standalone-`fn` emission in
  `codegen/program.rs` is the basis for the per-function codegen slice.
- `optimize_program` / `optimize_program_v2` (the ARCHITECT path, `optimize/mod.rs`) — the
  AOT optimizer the T4 slice runs (e-graph, supercompile, symmetry, loop-split).
- Differential harnesses (`crates/logicaffeine_tests/tests/jit_vm_differential.rs`,
  `runpath_opts.rs`) — the per-tier soundness gate.

---

## 15. Verification

- **Correctness:** `tier_invariance` (output byte-identical T0–T3) + `eager_equals_today`
  (T3 == legacy) are the load-bearing gates; run via `./scripts/run-all-tests-fast.sh`.
- **The win (speed):** the new Eager-vs-Tiered axis in `run-interp-vs-js.sh` quantifies the
  tiering's effect against the current number; studio latency is felt directly under
  `dx serve -p logicaffeine-web` from the repo root.
- **AOT-native (T4):** `tier_invariance` extends to T4 (byte-identical to baseline, and ==
  the `largo build` binary's output); a **fall-through** test proves an absent/stale bundle
  yields identical output via VM+JIT; the **3-bar benchmark** shows the win — desktop cdylib
  (full native) and browser wasm (near-native) against Node and C, toggled by "bundle native".
