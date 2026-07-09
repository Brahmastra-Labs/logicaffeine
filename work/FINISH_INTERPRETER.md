# FINISH_INTERPRETER — Guiding Specification

**Make Logos's concurrency, tasks, parallelism, and networking real across every execution tier and in the browser Studio — under one shared, deterministic, seed-driven semantic spec — without costing one instruction on the compiled hot-path (currently 2.55× C geomean).**

This document is the **executable specification** for that work. It is written to be followed test-first: every phase states its objective, the exact files it touches, the exact public surface it adds, the **RED tests written before any implementation**, the GREEN steps, and a binary **Definition of Done**. Work it top to bottom. Do not skip the regression gate between phases.

---

## 0. How to use this spec

### 0.1 The TDD loop (mandatory, per CLAUDE.md)

For every unit of work:

1. **RED** — write the failing test from this spec *first*. The test names and assertions are given below; copy them verbatim. Run it; confirm it fails for the right reason.
2. **GREEN** — write the minimum implementation to pass. Never edit a RED test to make it pass — if a test seems wrong, **stop and ask**.
3. **REFACTOR** — clean up with the test green.
4. **GATE** — run the phase's full test set + the **global regression gate** (§6). A failing test is always a regression; do not advance until everything is green.

### 0.2 Test runners

- Targeted RED/GREEN loop: `cargo nextest run -p <crate>` or `cargo nextest run -E 'test(<name>)'`.
- Skip-e2e fast path (no Z3): `cargo test --no-fail-fast -- --skip e2e > /tmp/test_file_logs.txt 2>&1; echo "EXIT: $?" >> /tmp/test_file_logs.txt`.
- Full suite (parity, all crates, Z3, doctests): `./scripts/run-all-tests-fast.sh` (`--no-ignored` while iterating). **Never run two suites at once.**
- Benchmarks (hot-path gate): the benchmark harness in `benchmarks/` + `crates/logicaffeine_tests/tests/common/mod.rs` differential helpers.

### 0.3 Definition of Done (applies to every phase)

A phase is Done **iff** all of:

- [ ] Every RED test listed for the phase is green.
- [ ] The global regression gate (§6) is green — including **byte-identical benchmark output** and **unchanged benchmark timings** for all non-concurrent programs.
- [ ] No RED test was modified to pass.
- [ ] New public items have doc comments; diagnostics use the project's Socratic style.
- [ ] `cargo clippy` clean on touched crates.

### 0.4 Naming conventions for new tests

| Layer | Location | Pattern |
|---|---|---|
| Classifier / analysis units | inline `#[cfg(test)]` in the module | `classify_*`, `send_check_*` |
| Scheduler core units | `crates/logicaffeine_runtime/src/*` inline + `tests/` | `sched_*`, `seed_*`, `replay_*` |
| Interpreter concurrency | `crates/logicaffeine_tests/tests/interp_concurrency.rs` | `interp_*` |
| VM concurrency | `crates/logicaffeine_tests/tests/vm_concurrency.rs` | `vm_*` |
| Cross-tier / cross-driver differential | `crates/logicaffeine_tests/tests/concurrency_differential.rs` | `diff_*` |
| AOT dual-mode equivalence | `crates/logicaffeine_tests/tests/concurrency_aot.rs` | `aot_*` |
| Browser (wasm) | `crates/logicaffeine_tests/tests/wasm_concurrency.rs` (wasm-bindgen-test) | `wasm_*` |
| TV | `crates/logicaffeine_tv/tests/concurrency_tv.rs` | `tv_*` |

The **corpus**: reuse the programs already in `crates/logicaffeine_tests/tests/e2e_concurrency.rs` and `phase54_concurrency.rs` (they already pass in compiled mode and assert known outputs). Every interpreter/VM/driver test should, wherever possible, **run a corpus program and assert the same output the compiled test already asserts** — this gives differential coverage for free.

---

## 1. Current state (ground truth)

Logos already has **full syntax, AST, and AOT compile-to-Rust** for concurrency/networking. What is missing is *runtime execution* on the interpreter/VM/JIT and in the browser.

| Capability | Tree-walker | VM | JIT | AOT (Rust) | Browser |
|---|---|---|---|---|---|
| `Concurrent` / `Parallel` | ❌ sequentialized (`interpreter.rs:1342`,`:2715`) | ❌ no opcode | ❌ | ✅ `tokio::join!`/`rayon` (`codegen/stmt.rs:2691`,`:2813`) | ❌ |
| `LaunchTask` / handle | ❌ rejected (`:1647`,`:2933`) | ❌ | ❌ | ✅ `tokio::spawn` (`:1932`) | ❌ |
| `CreatePipe` / `Send` / `Receive` / `Try*` | ❌ rejected | ❌ | ❌ | ✅ `mpsc` (`:1977`–`:2085`) | ❌ |
| `Select` / `After` | ❌ rejected | ❌ | ❌ | ✅ `tokio::select!` (`:2092`) | ❌ |
| `StopTask` | ❌ rejected | ❌ | ❌ | ✅ `.abort()` (`:2087`) | ❌ |
| Networking `Listen`/`ConnectTo`/`Sync` | ❌ rejected (`:1589`–`:1626`) | ❌ | ❌ | ✅ libp2p (`network/mesh.rs`) | ❌ |
| `Mount`/file I/O | ⚠️ async-only; **VFS never wired into browser interpreter** | n/a | n/a | ✅ | ❌ (VFS not handed to interp) |

Backing runtime already exists for the compiled path: `crates/logicaffeine_system/src/concurrency.rs` (`spawn` `:139`, `Pipe` `:171`, `check_preemption` `:288`) and `network/` (libp2p). CRDTs exist in `crates/logicaffeine_data/src/crdt/` and CRDT ops already run in the interpreter (`interpreter.rs:1432-1521`).

**AST reference** — `crates/logicaffeine_language/src/ast/stmt.rs:298-601`:

```
Concurrent { tasks }                          Parallel { tasks }
LaunchTask { function, args }                 LaunchTaskWithHandle { handle, function, args }
CreatePipe { var, element_type, capacity }    SendPipe { value, pipe }    ReceivePipe { var, pipe }
TrySendPipe { value, pipe, result }           TryReceivePipe { var, pipe }
StopTask { handle }                           Select { branches: Vec<SelectBranch> }
SelectBranch::Receive { var, pipe, body }     SelectBranch::Timeout { milliseconds, body }
Listen { .. }  ConnectTo { .. }  LetPeerAgent { .. }  Sync { .. }  Mount { .. }
```

### 1.1 Implementation status (reconciled 2026-06-26)

The table in §1 above describes the *starting* state. As of this reconciliation the
code is far ahead of the original checkboxes — most phases shipped. Authoritative
status:

| Phase | State | Evidence |
|---|---|---|
| 0 Determinacy classifier | ✅ shipped | `concurrency/classify.rs`, `concurrency_classify.rs` |
| 1 Re-entrancy (`SharedCtx`/`TaskState`) | ✅ shipped | `interp_task_state.rs` |
| 2 Scheduler core (`logicaffeine_runtime`) | ✅ shipped | `scheduler.rs`, `seed.rs`, `executor.rs` |
| 3 Tree-walker concurrency + VFS | ✅ shipped | async `execute_stmt`; `with_vfs` wired (`ui_bridge.rs:1826`, `studio.rs:1358`). Sync-path rejections (`interpreter.rs:3601-3781`) are the intended non-async fallback — unreachable for concurrency programs (routing diverts on `needs_async \|\| uses_scheduler`). |
| 4 Send / escape analysis | ✅ shipped | `concurrency_send_check.rs` |
| 5 VM concurrency opcodes | ✅ shipped (channels/tasks/select) | `vm/instruction.rs`, `vm_concurrency.rs`. **Gap:** networking + `Concurrent`/`Parallel` VM parity (Phase 9). |
| 6 JIT deopt-at-concurrency seam | ✅ shipped | `codegen/detection.rs` deny-list |
| 7 Work-stealing M:N | ✅ shipped | `executor.rs`, `concurrency_differential.rs` |
| 8 AOT dual-mode (A default / B seeded) | ✅ shipped | `codegen/{program,stmt}.rs`, `concurrency_aot.rs` |
| Networking (Listen/Connect/Sync/Mount/peer-msg) | ✅ shipped (interpreter path) | `interpreter.rs:2146-2242` + `net.rs` (native+wasm) + `relay*.rs`; `interp_networking.rs`, `wasm_interp_net.rs`, `wasm_relay.rs`, `wasm_opfs.rs` |
| 10 Stdlib bundling | ⚠️ partial | `loader.rs` `apply_prelude` triggers only `concurrency.md`; **net/io/crdt dormant**, env/file/random/time `.lg` unwired → see **Phase A** below |
| 9 Browser driver + Net + OPFS + Tasks UI | 🚧 mostly shipped | drive loop / Net / VFS / OPFS / no-freeze cooperative driver / headless `wasm_concurrency.rs` / WASM size gate (now in CI) shipped. **Mixed channels+networking FIXED**: a program using both routed to the scheduler-less async path and panicked "outside a scheduler context"; now runs on the async scheduler loop that services channel ops AND network awaits over the host reactor (`TaskStep::IoPending`→`BlockedIo`→`WaitingForIo`→`wake_io` after a reactor yield). Tests: `interp_pipe_and_network_run_together`, `scheduler::io_pending_*`. **Remaining:** Studio Tasks/Channels UI (co-agent blocked) |
| 11 TV (concurrency) | ✅ shipped | `logicaffeine_tv` `symexec.rs` channels + seeded `Select` + seed-sweep; `concurrency_tv.rs` (`tv_determinate_reuses_equivalence`, `tv_nondeterminate_same_seed_agrees`, `tv_seed_sweep_refinement`, `splitmix_matches_runtime_seed`) |
| 12 Web-Worker browser multicore | ✅ primitive verified / 🚧 app wiring | true-multicore WASM **proven**: `scripts/wasm-threads/` (single-file probe, no crate) + `scripts/test-wasm-threads.sh` — 4 node `worker_threads` (real OS threads = the headless analog of browser Web Workers) over one shared `WebAssembly.Memory`, atomic counter exact + a sense-reversing barrier that only releases under genuine concurrency. Determinism = the already-proven native `Cooperative`==`WorkStealing` equivalence. **Remaining:** wire the web app to a worker pool behind a default-off `parallel` feature + ship COOP/COEP headers after the CORP audit (browser-manual gate; see Phase 12 below) |
| 13 Browser WASM-JIT backend | ✅ shipped | `vm/wasm_jit.rs` emits a WebAssembly module per hot region (dispatch-loop control flow); host is **cfg-split** — `wasmi` natively (codegen oracle), the platform's real `WebAssembly` (V8) via `js_sys` on wasm32. Verified on real V8: `wasm_jit_browser.rs` (5 tests under node; differential vs VM + lossless i64↔BigInt across the full range) + `wasm_jit_differential.rs` (native, in the fast suite's PASS 1b) |

Two cross-cutting workstreams not in the original phase list (added 2026-06-26):

- **Phase A — Partial evaluator / Futamura soundness for async + stdlib.** ✅ shipped — `effects.rs` carries `nondet`/`concurrent`, `function_is_specialization_safe` gates `try_specialize_call`, `bta.rs` clobbers effect-bound vars to Dynamic, and deep specialization recurses pure sub-parts of `Concurrent`/`Parallel`/`Select`/`Zone` without reordering effects. `optimize/effects.rs`
  misclassifies `Select`/`LaunchTask`/`TryReceivePipe`/`Mount`/etc. as **Pure** (catch-all
  `_ => EffectSet::default()`), so `try_specialize_call` can specialize a function containing a
  concurrency construct, drop a static param that flows into it, and leave a dangling free var
  (`substitute_stmt` catch-all does not substitute into the concurrency stmt) → **miscompile**.
  Baseline fix: concurrency/networking effects are always-Dynamic opaque boundaries PE never
  specializes across (matches `PROPER_FUTAMURA.md` Sprint 19 `core_pe_dynamic_all_effects`);
  then deep specialization of pure sub-parts inside `Concurrent`/`Parallel`/`Select`/`Zone`
  without reordering/duplicating effects. **DoD:** `phase_pe_concurrency`, `phase_bta` additions,
  `effects.rs` units, Futamura projections over stdlib programs green; §6 gate green.
- **Phase B — Stdlib invisibly exposed (all modules), collision-safe, native==wasm.** ✅ shipped — `loader.rs` `apply_prelude` is demand-driven + declarer-wins over net/io/crdt + env/file/random/time, with derived per-module name sets; benchmark corpus stays byte-identical (`Cow::Borrowed`). Demand-driven
  + declarer-wins prelude in `loader.rs`: prepend module `M` iff source *references* one of `M`'s
  names AND does not itself *define* any (user defs shadow; benchmark corpus stays byte-identical).
  Triggers auto-derived from each module's own definitions; cover net/io/crdt + env/file/random/time.
  **DoD:** `stdlib_prelude_invisible` (corpus byte-identity guard first), loader units,
  `prelude_identical_native_and_wasm` extended; §6 gate green.

---

## 2. ⚡ Performance contract (non-negotiable)

1. **Non-concurrent programs are byte-identical.** All benchmarks are pure-compute; they contain none of the concurrency statements. Codegen changes only the *lowering of* those statements ⇒ identical Rust → identical bytecode → identical JIT tiering. The "concurrency ops are JIT-ineligible" rule (Phase 6) only affects bodies that *contain* those ops. **2.55× is preserved by construction**, and §6's benchmark gate proves it every phase.
2. **Determinism lives in the interpreter/VM tier, never in the AOT binary.** The seeded scheduler (`logicaffeine_runtime`) is for Studio + TV; it is never linked into the compiled binary. Interpreter and binary share a *spec*, not a *code path*.
3. **Dual-mode compiled runtime — fast is the default (compile-time gated):**
   - **Mode A — Free-running (DEFAULT):** `Builder::new_multi_thread().enable_all()`, raw `tokio::select!`, `rayon::join`, `Show` → stdout. No seed, no choice-log, no buffering. **Identical emission to today.** Genuine multicore.
   - **Mode B — Deterministic replay (OPT-IN: `largo build --deterministic` / runtime `LOGOS_SEED=…`):** `new_current_thread()` + paused virtual clock + seeded `logos_select!` + buffered `Show`. For TV/debug/replay only.
   - Default build emits **zero** determinism code. Optional `--dual` emits both behind a one-time startup env check (no loop-level cost).
4. **Scheduler is tier-agnostic and never in the hot loop.** It runs only at yield/block points, not per statement. A *task body* runs on its normal tier: compiled-AOT native (the 2.55× path; scheduler not linked), VM+JIT on native (M:N work-stealing, genuine multicore), or tree-walker (browser, until the deferred **Phase 13** WASM-JIT backend — the x86 copy-and-patch JIT can't run in WASM — and reference semantics).

---

## 3. Concurrency language semantics (normative)

This section is the single source of truth that the classifier, interpreter, VM, AOT, and TV must all agree with.

### 3.1 Surface syntax → AST (reference)

| Surface | AST | Meaning |
|---|---|---|
| `Attempt all of the following:` `<block>` | `Concurrent { tasks }` | run sub-statements concurrently, join all |
| `Simultaneously:` `<block>` | `Parallel { tasks }` | same semantics as `Concurrent`; a *hint* to prefer distinct workers |
| `Launch a task to f(a, b).` | `LaunchTask { function, args }` | spawn `f(a,b)`, fire-and-forget |
| `Let h be Launch a task to f(a).` | `LaunchTaskWithHandle { handle, function, args }` | spawn, bind handle `h` |
| `Let ch be a Pipe of Int.` (opt. `with capacity N`) | `CreatePipe { var, element_type, capacity }` | new FIFO bounded channel (default cap 32) |
| `Send v into ch.` | `SendPipe { value, pipe }` | blocking move-send |
| `Receive x from ch.` | `ReceivePipe { var, pipe }` | blocking receive, bind `x` |
| `Try to send v into ch.` | `TrySendPipe { value, pipe, result }` | non-blocking send; `result` = success bool |
| `Try to receive x from ch.` | `TryReceivePipe { var, pipe }` | non-blocking; bind `x` to value-or-Nothing |
| `Stop h.` | `StopTask { handle }` | cooperative-cancel task `h` |
| `Await the first of:` `Receive x from ch:` `<body>` `After N seconds:` `<body>` | `Select { branches }` | nondeterministic choice over ready branches |

> Confirm exact spellings against `e2e_concurrency.rs` before writing surface-level tests; prefer reusing corpus programs over inventing syntax.

### 3.2 Denotational semantics (the table classifier + TV consume)

| Construct | Denotation | Determinacy |
|---|---|---|
| `LaunchTask f(args)` | spawn a process computing `f` over a **moved copy** of `args` | **Determinate** (Kahn process) |
| `Pipe of T` | a FIFO edge, buffer capacity `cap` | Determinate |
| `Send v into ch` | append `v` to channel history; block if full | **Determinate** (monotone on history) |
| `Receive x from ch` | pop head; block if empty | **Determinate** (FIFO order fixed) |
| `Parallel` / `Concurrent` | fork-join of branches; results joined | Determinate **iff** branches are data-independent (no two write/read the same channel with conflicting roles; no shared non-CRDT mutable) |
| `Select` (`Await the first of`) | **nondeterministic** choice over the *ready* branch set | **FORCES nondeterminism** |
| `After d:` (timeout branch) | a timer event racing other branches | **FORCES nondeterminism** |
| `Try to send/receive` | success depends on instantaneous buffer occupancy | **FORCES nondeterminism** |
| `StopTask h` | cancel at `h`'s next scheduling point | **FORCES nondeterminism** |
| CRDT shared cell + `Merge` | monotone join on a semilattice | Determinate (order-independent convergence) |

**Determinate fragment** = programs using only the Determinate rows. By **Kahn (1974)** their observable output is scheduling-independent. **Nondeterminate fragment** = any program reaching a "FORCES" construct.

### 3.3 Memory model (enforced by the Send/escape analysis, Phase 4)

- Each task owns an **isolated heap** of `RuntimeValue` (`Rc<RefCell<…>>`). No `Rc` is shared across tasks.
- Cross-task influence is exactly two channels:
  1. **Channels** — payload is **moved** out of the sender's heap (materialized to `RtPayload`, a `Send` value subset) and rebuilt into the receiver's heap.
  2. **CRDT/atomic cells** — `Arc`-backed shared values mutated only by monotone `Merge`/bump.
- A raw mutable `List`/`Set`/`Map`/`Struct` captured-and-mutated across a task boundary is a **compile error** ("pass it through a Pipe or make it a CRDT").

### 3.4 Seed / trace format (stable public contract — `logicaffeine_runtime::seed`)

```rust
pub struct SchedSeed(pub u64);
pub struct SeededRng { state: u64 }                 // SplitMix64; deterministic; WASM-safe

pub enum ChoiceKind { TaskPick, SelectWinner, ChanWaiterWake, TimerTieBreak, WorkerPlacement }

/// One recorded decision — id-agnostic, so the trace format never depends on
/// TaskId/ChanId/etc. (kept simple on purpose; the kind + option count are enough
/// for deterministic replay and divergence detection).
pub struct ChoicePoint { pub kind: ChoiceKind, pub options: usize, pub chosen: usize }

pub struct SchedTrace { pub seed: SchedSeed, pub choices: Vec<ChoicePoint> }

/// The choke point. Record mode draws from the RNG and logs the choice; replay
/// mode returns the recorded choice and panics if the live decision's shape
/// (kind/option count) diverges from what was recorded.
pub enum Chooser {
    Record { rng: SeededRng, seed: SchedSeed, choices: Vec<ChoicePoint> },
    Replay { trace: SchedTrace, pos: usize },
}
impl Chooser { pub fn decide(&mut self, kind: ChoiceKind, options: usize) -> usize { /* … */ } }
```

Every nondeterministic branch routes through **one** function — `Chooser::decide(&mut self, kind, options) -> usize` — which (a) draws from `SeededRng`, (b) records a `ChoicePoint`. In **replay mode** `decide` reads the next recorded `ChoicePoint` and asserts the kind + option count match (divergence detection). The `Scheduler` embeds a `Chooser`; this is the only place nondeterminism is resolved, and the auditable choke point TV and replay depend on. *(Implemented & green in Phase 2a / T4.)*

Public entry points the interpreter, VM, and TV all use:

```rust
pub fn run_with_seed(program: …, seed: SchedSeed) -> (Outputs, SchedTrace);
pub fn run_with_trace(program: …, trace: &SchedTrace) -> Outputs;
```

---

## 4. Architecture summary

- **One scheduler abstraction, two drivers:** `Cooperative` (M:1, browser/WASM, on the event loop) and `WorkStealing` (M:N, native multicore). Identical task/channel/select semantics; the seed governs all choices; in **serialized-decision mode** both drivers produce identical traces/outputs.
- **New crate `crates/logicaffeine_runtime/`** holds the scheduler core, generic over an opaque `Task` (knows nothing of `RuntimeValue`/`Op`). Modules: `scheduler.rs`, `executor.rs`, `task.rs`, `channel.rs`, `select.rs`, `timer.rs`, `seed.rs`, `payload.rs`, `trace.rs`, `waker.rs`.
  - **Charter (and why it is its own crate, not `system` or `compile`):** the *deterministic concurrency runtime* for the interpreter & VM — pure, WASM-safe, **tokio-free** (M:N uses `std::thread` + crossbeam, not tokio). It is **never linked into AOT binaries** (invariant I6), which is exactly what distinguishes it from `logicaffeine_system` (the tokio/libp2p **platform** services used by *both* the interpreter and compiled programs). The crate boundary *enforces* its purity (you cannot import tokio or `RuntimeValue` into it) — that enforcement is the point. It is the only net-new active crate; everything else folds into existing crates.
- **`RtPayload`** (in `logicaffeine_runtime::payload`) is the `Send` value subset crossing task/thread boundaries; `materialize`/`rebuild` marshalling lives in `logicaffeine_compile` (knows `RuntimeValue`).
- **Cross-target platform services extend the EXISTING `logicaffeine_system` crate** (no new crates, no `system::runtime`): the `Spawner`/`Timer`/`Channel`/`Yield` traits fold into `system::concurrency` (beside the current tokio `spawn`/`Pipe`/`check_preemption`), and the `Net` trait + transports into `system::network` (beside the libp2p mesh). Native (tokio/libp2p) + browser (wasm-bindgen/gloo-net) impls, mirroring the existing `Vfs` Send/`?Send` split.

---

## 5. Phases (test-first)

> Build order is strict: each phase's RED tests assume the prior phases are green. The **global regression gate (§6) runs after every phase.**

### Phase 0 — Memory-model spec + determinacy classifier

**Objective.** A pure, fast AST analysis that labels any program *Determinate* or *Nondeterminate*, with witnesses. No behavior change. This powers diagnostics, AOT mode selection, and TV gating.

**Files.**
- New: `crates/logicaffeine_compile/src/concurrency/mod.rs`, `model.rs`, `classify.rs`.
- New doc: `crates/logicaffeine_compile/docs/CONCURRENCY_MODEL.md` (copy §3 verbatim as the canonical model).
- Modify: `crates/logicaffeine_compile/src/lib.rs` (module wiring), `codegen/mod.rs` (re-export).

**Public surface.**
```rust
pub enum Determinacy { Determinate, Nondeterminate { witnesses: Vec<NondetWitness> } }
pub struct NondetWitness { pub kind: NondetKind, pub span: Span }
pub enum NondetKind { Select, AfterTimer, TryRecv, TrySend, StopTask }

/// Whole-program, transitive through Call / LaunchTask targets.
pub fn classify_program(stmts: &[Stmt]) -> Determinacy;
/// Branch-independence check for Parallel/Concurrent (shares alias machinery
/// with codegen::detection::collect_mutable_vars / collect_escaping_collection_vars).
pub fn branches_independent(tasks: &Block) -> bool;
```
Shape the recursion exactly like `codegen/detection.rs::requires_async` (`detection.rs:24-63`) and reuse its transitive-call walk (`:480-587`).

**RED tests** (`crates/logicaffeine_compile/src/concurrency/classify.rs` `#[cfg(test)]`):
- `classify_producer_consumer_is_determinate` — a `LaunchTask`+`Pipe`+`Send`+`Receive` program ⇒ `Determinate`.
- `classify_select_is_nondeterminate` — any `Await the first of:` ⇒ `Nondeterminate` with a `NondetKind::Select` witness at the right span.
- `classify_after_timeout_is_nondeterminate` — `After N seconds:` ⇒ witness `AfterTimer`.
- `classify_try_send_and_try_receive_are_nondeterminate` — each ⇒ the matching `Try*` witness.
- `classify_stop_task_is_nondeterminate`.
- `classify_transitive_through_launch` — `f` is determinate but `LaunchTask g` where `g` contains a `Select` ⇒ whole program `Nondeterminate`.
- `classify_parallel_independent_branches_determinate` vs `classify_parallel_shared_channel_dependent` — branch-independence.
- Drive every program in `e2e_concurrency.rs` / `phase54_concurrency.rs` through `classify_program` and assert the expected label per `corpus_determinacy_table` (a fixture listing each program → expected `Determinacy`).

**GREEN steps.** Implement `model.rs` (the denotation table as data) and `classify.rs` (AST walk + transitive closure + branch-independence via existing alias helpers).

**Definition of Done.** All `classify_*` green; §6 gate green (trivially — no behavior change).

---

### Phase 1 — Interpreter re-entrancy refactor (`SharedCtx` / `TaskState`)

**Objective.** Make the tree-walker re-entrant so multiple task continuations can run without aliasing `&mut Interpreter`. **No concurrency yet** — purely structural; the single-task case is "one `TaskState`".

**Files.** `crates/logicaffeine_compile/src/interpreter.rs` (large, mechanical).

**Design (as implemented — DONE).** `Interpreter` becomes `{ ctx: SharedCtx, task: TaskState, pub output }`:
- `TaskState` — per-task execution state: `env`, `call_depth`, the TCO/repeat scratch (sync + async). Owned per task.
- `SharedCtx<'a>` — `interner`, `functions`, `struct_defs`, `closure_bodies`, `policy_registry`, `kernel_ctx`, `output_callback`, the pre-interned builtin symbols, `vfs`, (later) `net`. Immutable-after-setup.
- **Methods stay `&mut self` — no signature change.** Re-entrancy comes NOT from threading `(state, ctx)` through every signature (the original Phase 1b — **obviated**) but from the **`Rc<SharedCtx>` sharing model**: each concurrent task is a *separate `Interpreter` sharing one `Rc<SharedCtx>`*, each owning its own `TaskState`, so their futures never alias. This collapses Phase 1b into a one-line wrapping step done when the scheduler spawns tasks (Phase 3a). The migration was purely mechanical field-grouping (~225 `self.X` → `self.ctx.X` / `self.task.X`).

**RED tests.** **No new tests.** The acceptance criterion is **the entire existing suite stays green** — this is the regression proof for a structural refactor. Add one focused unit `interp_single_task_state_roundtrip` asserting a trivial program still produces identical output through the new `TaskState` entry.

**GREEN steps.** Introduce the structs, thread them through, keep one `TaskState` at the top. Land incrementally behind a green suite.

**Definition of Done.** Full `./scripts/run-all-tests-fast.sh` green; benchmark gate green; the new entry signature in place and used by `ui_bridge`.

---

### Phase 2 — Scheduler core crate (`logicaffeine_runtime`)

**Objective.** The deterministic, seed-driven cooperative scheduler, fully unit-tested in isolation against *toy* tasks (no interpreter coupling). Proves determinism + replay before any wiring.

**Files.** New crate `crates/logicaffeine_runtime/` (workspace member): `lib.rs`, `scheduler.rs`, `executor.rs`, `task.rs`, `channel.rs`, `select.rs`, `timer.rs`, `seed.rs`, `payload.rs`, `trace.rs`, `waker.rs`, `Cargo.toml` (features: `cooperative` default; `work_stealing` native-only). Add to root `Cargo.toml` workspace members.

**Public surface.** `Task` trait (`poll(&mut self, ctx) -> TaskStep`), `TaskStep::{Yield, BlockedOnRecv, BlockedOnSend, BlockedOnSelect, BlockedOnTimer, Exit}`, `Scheduler`, `Cooperative` executor, the §3.4 seed/trace types, `RtPayload` (the `Send` enum), `Chan`/`ChanId`/`TaskId`/`TimerId`, `run_with_seed`/`run_with_trace`.

**Scheduling policy (annotatable, with a nice default).** The ready-task selection discipline is a first-class, configurable `SchedulePolicy`, bundled with the other runtime knobs in a `SchedulerConfig` that has a `Default` and a fluent builder, and is later settable per-program via a `## Scheduler: <policy>` decorator (the same decorator family as `## No <opt>` / `## NoPrelude`).

```rust
pub enum SchedulePolicy {
    Fifo,        // deterministic FIFO ready queue — fair, predictable;  THE DEFAULT
    Lifo,        // stack discipline — depth-first, better locality, can starve
    RoundRobin,  // explicit rotation with a fixed cooperative quantum
    Random,      // seeded-random ready pick — explores interleavings (fuzz/race-finding)
    Priority,    // highest task priority first, FIFO within a priority
}
impl Default for SchedulePolicy { fn default() -> Self { SchedulePolicy::Fifo } }

pub struct SchedulerConfig {
    pub policy: SchedulePolicy,          // default: Fifo
    pub clock: ClockMode,                // Logical (default; tests/TV) | Wall (real sleeps)
    pub default_channel_capacity: usize, // default: 32 (matches the AOT mpsc default)
    pub preempt_every: u32,              // cooperative yield interval (steps/quantum)
    pub workers: usize,                  // M:N worker count (1 = cooperative)
}
impl Default for SchedulerConfig { /* Fifo + Logical + 32 + sane preempt + workers=1 */ }
```

Determinism holds under *every* policy: `Fifo`/`Lifo`/`RoundRobin`/`Priority` are deterministic by construction; `Random` resolves its pick through the same seeded `Chooser::decide`, so it too is reproducible under a fixed seed. The default (`Fifo` + `Logical` clock) is the fair, predictable, fully-deterministic baseline; annotations opt into the others (e.g. `Random` for race-exposing fuzz runs, `Priority` for latency-sensitive tasks).

**RED tests** (`crates/logicaffeine_runtime/tests/`):
- `seed_rng_is_deterministic` — `SeededRng` from the same seed yields the same stream; different seeds differ.
- `sched_producer_consumer_fifo` — toy producer sends `0..N` into a bounded channel; consumer receives; asserts FIFO order and full drain.
- `sched_bounded_channel_blocks_sender` — capacity-1 channel: sender blocks until receiver drains (assert via interleaving counter).
- `sched_select_winner_is_seeded` — two ready branches; `decide` picks the seed-determined winner; the same seed always picks the same branch; a recorded `SchedTrace` contains a `SelectWinner`.
- `sched_timer_wheel_orders_logically` — two timers; under `LogicalClock` they fire in armed order with seed tie-break, zero wall-clock dependence.
- `replay_roundtrip_is_bit_identical` — `let (out, tr) = run_with_seed(p, s); assert_eq!(run_with_trace(p, &tr), out)`.
- `replay_divergence_panics_on_option_mismatch` — feeding a trace whose option set no longer matches panics with a clear message.
- `seed_sweep_is_reproducible` — for seeds `{0,1,2,7,42}`, repeated `run_with_seed` is identical each time.
- `deadlock_is_deterministic` — all tasks blocked, no timers ⇒ a stable `Deadlock` outcome (an observable, not a hang).
- `policy_default_is_fifo` — `SchedulerConfig::default().policy == SchedulePolicy::Fifo`.
- `sched_fifo_runs_in_spawn_order`, `sched_lifo_runs_newest_first`, `sched_roundrobin_rotates`, `sched_priority_orders_by_priority`, `sched_random_is_seed_reproducible` — each policy's ready-order behavior; `Random` reproducible under a fixed seed; all five deterministic given `(seed, policy)`.

**GREEN steps.** Implement the run loop, the `decide` choke point, the channel wait-sets, the hierarchical timer wheel with `LogicalClock`/`WallClock` strategies, and the replay reader.

**Definition of Done.** All `sched_*`/`seed_*`/`replay_*` green; crate builds for `wasm32-unknown-unknown` (CI build-target check — no tokio, no threads).

---

### Phase 3 — Tree-walker concurrency (cooperative) + browser VFS unblock

**Objective.** The interpreter executes the **determinate fragment** (and `Select`, seeded) on the cooperative driver. Replace every reject/sequentialize site. Wire the Studio's VFS into the interpreter.

**Files.** `crates/logicaffeine_compile/src/interpreter.rs`, `ui_bridge.rs`; marshalling helpers; `studio.rs` (pass `WebVfs`).

**Design.**
- Add `RuntimeValue::Chan(ChanId)` and `TaskHandle(TaskId)` (Copy tokens; identity arms in `PartialEq`/`Hash` at `interpreter.rs:425`/`:449`).
- `materialize(&RuntimeValue) -> Result<RtPayload, SendError>` (reuse `deep_clone` `:509`; sole-owner `Rc::strong_count==1` fast path) and `rebuild(RtPayload) -> RuntimeValue`.
- `Interpreter`/`SharedCtx` holds `scheduler: Option<Rc<RefCell<Scheduler>>>`.
- Replace: `:1342`/`:2715` (`Concurrent`/`Parallel` → spawn children + join), `:1647`/`:2933` (Launch/Pipe/Send/Receive/Try/Stop/Select → scheduler ops), reroute `Sleep` (`:1606`) to a scheduler timer. Extend `needs_async` (`:4027`) so any concurrency/networking statement forces the async executor.
- `run_treewalker` (`ui_bridge.rs:1507`) constructs a `Scheduler` + `Cooperative` executor instead of bare `block_on` (`:1520`).
- **VFS unblock:** `interpret_streaming` / `interpret_for_ui*` accept and `.with_vfs(...)` the VFS; `studio.rs:1307` passes its `WebVfs` (`:1111`).

**RED tests** (`crates/logicaffeine_tests/tests/interp_concurrency.rs`, run via `run_interpreter`):
- `interp_launch_and_join_matches_compiled` — a corpus `LaunchTask` program; assert the **same output** the compiled e2e test asserts.
- `interp_pipe_producer_consumer` — send `1..=5`, receive and `Show`; assert `1 2 3 4 5`.
- `interp_bounded_pipe_backpressure` — capacity-1; assert it completes (no deadlock) and order preserved.
- `interp_select_receive_vs_timeout_seeded` — `Await the first of: Receive … / After …`; under a fixed seed the winner is stable and the output matches across repeats.
- `interp_try_receive_empty_is_nothing` and `interp_try_send_full_reports_false`.
- `interp_stop_task_aborts` — a launched task is `Stop`ped; assert it produced no further output after the abort point.
- `interp_concurrent_block_runs_all` and `interp_parallel_block_runs_all` — assert all branch outputs present.
- `interp_deadlock_detected` — a receive with no sender ⇒ deterministic deadlock error (assert the message).
- `interp_seed_determinism` — same program + same seed ⇒ identical output across runs (and identical `SchedTrace`).
- `wasm_mount_read_write_roundtrip` (in `wasm_concurrency.rs`, wasm-bindgen-test) — `interpret_streaming` mounts, writes, reads back through the now-wired VFS.

**GREEN steps.** Implement the statement lowerings against the Phase-2 scheduler; wire VFS.

**Definition of Done.** All `interp_*` + the wasm VFS test green; every corpus program that is in the determinate fragment produces the compiled-mode output through the interpreter; §6 gate green.

---

### Phase 4 — Send / escape static analysis

**Objective.** Make Phase 3 *sound* and unblock M:N: reject programs that share a raw mutable heap across tasks; require channel payloads to be `Send`-materializable.

**Files.** New `crates/logicaffeine_compile/src/analysis/send_check.rs`; wire into the pre-run/pre-compile stage (same point as `needs_async`) so every tier rejects uniformly with one diagnostic.

**Public surface.**
```rust
pub struct SendDiagnostic { pub span: Span, pub message: String }   // Socratic style
pub fn check_send_escape(stmts: &[Stmt]) -> Result<(), Vec<SendDiagnostic>>;
```
Checks (§3.3): channel-payload materializability; spawned-body free-variable discipline (moved-by-value / CRDT / read-only Copy); use-after-send lint→error; `Parallel` shared non-CRDT mutable = data race.

**RED tests** (inline + `interp_concurrency.rs`):
- `send_check_accepts_message_passing` — producer/consumer + CRDT programs pass.
- `send_check_rejects_shared_mutable_list` — a `List` captured-and-mutated by a task while the parent retains it ⇒ error with the right span and the "pass it through a Pipe or make it a CRDT" message (snapshot).
- `send_check_rejects_non_send_channel_payload` — `Pipe of Function`/closure-with-non-Send-capture ⇒ error.
- `send_check_warns_then_errors_use_after_send` — value used after being sent ⇒ diagnostic.
- `send_check_rejects_parallel_shared_mutable` — two `Parallel` branches writing the same non-CRDT var ⇒ refuse to compile.
- `send_check_allows_crdt_shared_cell` — a CRDT shared across tasks passes.

**Definition of Done.** All `send_check_*` green; diagnostics snapshot-stable; the analysis runs on every tier; §6 gate green.

---

### Phase 5 — VM concurrency opcodes

**Objective.** The VM tier executes concurrent programs: new opcodes, a resumable run loop, per-task `Vm`.

**Files.** `crates/logicaffeine_compile/src/vm/{instruction.rs,machine.rs,compiler.rs,value.rs,nanbox.rs}`.

**New `Op` variants.** `Spawn`/`SpawnHandle`/`TaskAbort`/`TaskAwait`; `ChanNew`/`ChanSend`/`ChanRecv`/`ChanTrySend`/`ChanTryRecv`/`ChanClose`; `SelectBegin`/`SelectRecv`/`SelectTimeout`/`SelectCommit`; `TimerArm`; `Yield`. nanbox tags for `Chan`/`TaskHandle`/`Timer` (small ids). Operand/stack semantics per the design (register machine; blocking ops save `pc` and return to the scheduler).

**Design.** Restructure `Vm::run` (`machine.rs:1330`) into `run_until_block() -> VmStep::{Done, Blocked(reason, resume_pc), Yielded}`. The `Vm` instance *is* the task heap and persists across polls. Each task = a distinct `Vm` (distinct registers/heap). `VmCompiler` (`vm/compiler.rs:273`) gains arms for the 9 statements; remove the blanket `Err("vm: unsupported")` (`:1837`) — the only remaining rejection is a Send-analysis failure.

**RED tests** (`crates/logicaffeine_tests/tests/vm_concurrency.rs`):
- `vm_launch_and_join`, `vm_pipe_producer_consumer`, `vm_bounded_pipe_backpressure`, `vm_select_seeded`, `vm_try_recv_empty`, `vm_stop_task`, `vm_concurrent_block`, `vm_deadlock_detected` — mirror the `interp_*` set, asserting the **same outputs**.
- `diff_treewalker_eq_vm_seeded` (`concurrency_differential.rs`) — for every corpus program and seeds `{0,1,2,7,42}`, `run_interpreter(seed) == run_vm(seed)` byte-for-byte.

**Definition of Done.** All `vm_*` + `diff_treewalker_eq_vm_seeded` green; §6 gate green (VM bytecode for non-concurrent programs unchanged — snapshot a couple).

---

### Phase 6 — JIT deopt-at-concurrency seam

**Objective.** Keep the JIT integer-fast; concurrency ops are never compiled. Confirm hot integer loops *inside* tasks still tier.

**Files.** `crates/logicaffeine_compile/src/codegen/detection.rs` (eligibility deny-list), `crates/logicaffeine_compile/src/vm/native_tier.rs` (defensive `DeoptAt` reuse). No new stencils in `logicaffeine_jit`/`logicaffeine_forge`.

**Design.** Add every concurrency `Op` to the region/function-ineligible set. Native regions are *yield-free* by construction (integer-only ⇒ no channel/timer ops). Defensive boundary case reuses `RegionOutcome::DeoptAt { resume_pc }` (`native_tier.rs:98`).

**RED tests** (`vm_concurrency.rs`):
- `jit_hot_loop_inside_task_still_tiers` — a launched task with a hot integer loop; assert it JIT-tiers (via the existing tier-counter/inspection hook) and produces the correct result.
- `jit_body_with_chan_op_not_selected` — a body containing `ChanSend` is never region-selected (assert via the detector).
- `jit_benchmark_tiering_unchanged` — a representative benchmark's tiering decisions are identical to baseline (snapshot).

**Definition of Done.** All three green; §6 benchmark gate green (timings unchanged is the real proof here).

---

### Phase 7 — Work-stealing native driver (M:N)

**Objective.** Genuine multicore on native: N workers, each a full interpreter/VM over its own heap; `Send`-only handoff; serialized-decision mode for determinism.

**Files.** `crates/logicaffeine_runtime/src/executor.rs` (`WorkStealing`), cross-thread queues (reuse `logicaffeine_system::concurrency` tokio/crossbeam plumbing), `payload.rs` (ensure `RtPayload: Send`).

**RED tests** (`concurrency_differential.rs`):
- `diff_cooperative_eq_workstealing_seeded` — for every corpus program and the seed sweep, `Cooperative(seed)` output == `WorkStealing-serialized(seed)` output byte-for-byte. **This is the load-bearing "same semantics on both drivers" test.**
- `workstealing_actually_parallel` — a CPU-bound `Parallel` of independent pure branches completes with wall-time < sum-of-branches on a multicore box (a throughput smoke test, generous threshold).
- `workstealing_payload_is_send` — compile-time assertion (`fn _assert_send<T: Send>(){}` over `RtPayload`).

**Definition of Done.** Cross-driver equivalence green over the full corpus + seed sweep; §6 gate green.

---

### Phase 8 — AOT dual-mode runtime (the performance-contract phase)

**Objective.** Default AOT emission stays byte-identical to today (Mode A). Opt-in deterministic Mode B exists and is seed-faithful to the interpreter.

**Files.** `crates/logicaffeine_compile/src/codegen/{program.rs,stmt.rs,detection.rs}`, `crates/logicaffeine_system/src/concurrency.rs` (`bootstrap`, `logos_select!`, `logos_spawn`, buffered `Show`), `crates/logicaffeine_tests/tests/common/mod.rs` (seeded harness + emitted Cargo.toml features).

**Design.** `program.rs:539` keeps the `requires_async` gate; default prologue unchanged. A `requires_seeded_runtime` gate (Nondeterminate **and** `--deterministic`) emits the Mode-B prologue. `logos_select!` delegates to raw `tokio::select!` in production and to a seeded deterministic pick in seeded mode — sharing the **same choice function** as the interpreter and TV.

**RED tests** (`crates/logicaffeine_tests/tests/concurrency_aot.rs`):
- `aot_default_codegen_for_benchmark_is_byte_identical` — emit Rust for a representative benchmark; assert it equals the committed baseline snapshot. **Guards the hot-path.**
- `aot_default_codegen_for_concurrent_program_unchanged_modeA` — a concurrent program in Mode A emits today's tokio shape (snapshot).
- `aot_determinate_equals_interpreted` — for every Determinate corpus program, `assert_compiled_equals_interpreted` (no seed; justified by Kahn).
- `aot_nondeterminate_same_seed_equivalence` — for every Nondeterminate program and seeds `{0,1,2,7,42}`, `assert_compiled_equals_interpreted_seeded(src, seed)` byte-for-byte (Mode B both sides).
- `aot_production_output_in_allowed_set` — Mode A multi-thread output ∈ the interpreter's seed-swept allowed-set (refinement smoke).

**Definition of Done.** All `aot_*` green; **`aot_default_codegen_for_benchmark_is_byte_identical` green is mandatory**; §6 benchmark timings unchanged.

---

### Phase 9 — Browser driver + `Net` + OPFS buffers + Studio Tasks UI

**Objective.** Concurrent programs run in the Studio without freezing the UI; full browser networking behind a `Net` trait; OPFS-backed buffers.

**Files.** `crates/logicaffeine_compile/src/ui_bridge.rs` (drive loop + macrotask yields), `crates/logicaffeine_system/src/concurrency/{mod,native,wasm}.rs` (cross-target traits folded beside the existing tokio primitives), `network/net.rs` (`Net` + `NativeNet` + `BrowserNet`) + `ws_relay.rs`, `interpreter.rs` (replace networking rejections `:1589-1626`, `.with_net`), `apps/logicaffeine_web/src/ui/pages/studio.rs` (Tasks/Channels strip), `apps/logicaffeine_web/Cargo.toml`, optional Cloudflare relay Worker.

**Design.**
- Cooperative drive loop yields a macrotask (`gloo_timers::future::TimeoutFuture::new(0).await`, proven at `interpreter.rs:1619`) after each tick so Dioxus repaints; `check_preemption` (10 ms) is the in-loop hook; interleaved task `Show` streams via the existing `OutputCallback`.
- `Net` trait (Send native / `?Send` wasm) with `listen`/`connect`/`send`/`subscribe`/`publish`/`next_event`. **Native** = adapter over `mesh.rs`. **Browser v1** = `gloo-net` WS + fetch; `Listen`/`Sync`/gossip lower to a WS relay/rendezvous; received deltas feed `MergeCrdt`. **Browser v2+ (feature-gated)** = WebTransport, WebRTC, `net-libp2p-wasm` — never in the default bundle (payload budget vs 14.7 MB).
- OPFS staging for large payloads via `WorkerOpfsVfs` (`worker_opfs.rs:56,105`); OPFS→IndexedDB fallback preserved.

**RED tests** (`wasm_concurrency.rs`, wasm-bindgen-test, `wasm-pack test --headless`):
- `wasm_three_task_pipeline_streams_without_freezing` — 3-task producer/consumer streams interleaved output; assert all lines arrive and the event loop yields between ticks (drive a fake timer).
- `wasm_connect_and_echo_over_ws` — `ConnectTo`/`send` against a mock WS echo server; assert round-trip.
- `wasm_sync_crdt_merges_received_delta` — a received gossip delta merges into a CRDT and updates output.
- `wasm_mount_buffer_through_opfs` — a large payload stages through OPFS, not linear memory.
- Native parity: `interp_listen_connect_send_roundtrip` (loopback `NativeNet`) in `interp_concurrency.rs`.

**Definition of Done.** wasm tests green in headless CI; native `Net` round-trip green; default WASM size within budget of 14.7 MB (assert in a size-gate test); §6 gate green.

---

### Phase 10 — Stdlib bundling

**Objective.** Concurrency/net/IO/CRDT vocabulary uniformly available, embedded into the binary/WASM, zero round-trips.

**Files.** New `crates/logicaffeine_compile/assets/std/{concurrency,net,io,crdt}.md`; `crates/logicaffeine_compile/src/loader.rs` (new `logos:` intrinsics + a `prelude()` concatenator); `ui_bridge.rs` `with_parsed_program` (`:972`) prelude prepend with a `## NoPrelude` escape hatch.

**RED tests** (inline + `interp_concurrency.rs`):
- `prelude_is_embedded_and_loads` — `prelude()` returns the concatenated modules; parses clean.
- `prelude_auto_prepended_enables_vocabulary` — a program using a stdlib concurrency helper works **without** an explicit import.
- `prelude_no_prelude_decorator_opts_out` — `## NoPrelude` disables prepend.
- `prelude_identical_native_and_wasm` — the embedded bytes are identical across targets (compile-time `include_str!` equality).

**Definition of Done.** All `prelude_*` green; §6 gate green.

---

### Phase 11 — TV extension (translation validation for concurrency)

**Objective.** Determinate fragment reuses the existing strong equivalence (zero new SMT). Nondeterminate fragment gets seeded replay equivalence + a seed-sweep refinement; optional SMT `Select` encoding.

**Files.** `crates/logicaffeine_tv/src/{lib.rs,symexec.rs,verdict.rs}`, `crates/logicaffeine_tests/tests/common/mod.rs`.

**Design.** Gate `check_encoder_sound` (`tv/lib.rs:44`) on the classifier: Determinate ⇒ existing ordered-output equality (now justified for all schedules by Kahn; add the precondition + doc). Nondeterminate ⇒ `check_seeded_sound(source, seed)` with both sides pinned to `seed`; extend `symexec.rs` to encode `Send`/`Receive` (channel histories) and `Select` (seeded `ite` over branch-ready predicates — the **same** choice function as the runtime). `verdict.rs` gains `SeedReplayAgrees`/`SeedReplayDisagrees`/`OutcomeSetRefined`; `Try`/wall-clock `After` stay honestly `Unsupported`.

**RED tests** (`crates/logicaffeine_tv/tests/concurrency_tv.rs`):
- `tv_determinate_reuses_equivalence` — a Determinate program passes the existing equivalence with no new obligations.
- `tv_nondeterminate_same_seed_agrees` — a `Select` program: interpreter(seed) ≡ encoder(seed) for the seed sweep.
- `tv_seed_sweep_refinement` — compiled outcomes across seeds ⊆ interpreter allowed-set.
- `tv_try_marked_unsupported` — a `Try`-based program is honestly reported `Unsupported`, not falsely proven.

**Definition of Done.** All `tv_*` green; the determinate path adds zero SMT obligations (assert via the report); §6 gate green.

---

### Phase 12 — Web Worker real parallelism in the browser

**Objective.** True multicore in the browser behind a feature flag, after a CORP audit.

**✅ Primitive verified (the load-bearing part).** Genuine multicore WebAssembly is real and
proven here, not hand-waved — `scripts/wasm-threads/` (a single-file probe, no crate) +
`scripts/test-wasm-threads.sh`:
- A `#![no_std]` module built for wasm32 with **`+atomics` + shared, imported memory** (a raw
  `rustc --crate-type=cdylib` — the atomic ops inline, so stock `core` suffices: no `build-std`,
  no nightly; wasm-ld `--shared-memory --import-memory`) — the *exact* build a browser Web-Worker
  pool needs, in an ~800-byte module.
- Driven by node `worker_threads` (real OS threads — the headless analog of browser Web
  Workers): N workers instantiate the **same** module against **one** shared
  `WebAssembly.Memory`. A contended atomic counter must total exactly `N*iters` (atomicity over
  shared memory), and a **sense-reversing barrier** that can only release if N threads make
  progress *at once* gates the pass — serial execution would spin out and fail loudly. Result:
  `workers=4 iters=1000000 → counter 4000000 OK, barrier all cleared`.

**Determinism is already proven.** A worker-driven run must equal the cooperative run
byte-for-byte on the same seed — this is the native `Cooperative`==`WorkStealing` equivalence
(`concurrency_differential.rs`), since all scheduling decisions go through one `Scheduler::decide`
regardless of *where* task compute runs. The browser pool inherits it by reusing that decision path.

**🚧 Remaining (browser-manual gate — needs a real browser + CORP audit).** Wire the web app to a
worker pool behind a default-off `parallel` feature (`crates/logicaffeine_system/src/concurrency/wasm.rs`
spawner, `apps/logicaffeine_web/public/assets/worker-pool.js`, `apps/logicaffeine_web/Cargo.toml`
`parallel`), and ship the cross-origin-isolation headers production needs:
```
# apps/logicaffeine_web/public/_headers — ONLY after the CORP audit; flipping these blind
# breaks every cross-origin subresource lacking CORP/CORS (license/registry fetches, fonts).
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
```
**Gate.** Staged on a throwaway Pages project; a CORP-compliance audit of every cross-origin
subresource must pass first. Default remains cooperative-only.

**Definition of Done.** Behind a default-off feature; the default Studio build and all prior gates
remain green and unchanged; the verified multicore primitive above is the parallel substrate.

---

### Phase 13 — Browser WASM-JIT backend ✅ shipped

**Status (2026-06-26).** Shipped + verified on a real engine. It lives in `crates/logicaffeine_compile/src/vm/wasm_jit.rs` (the VM crate, not `forge` — `forge` is `#![cfg(not(wasm32))]` and this backend must build for wasm32), behind the default-off `wasm-jit` feature. The emitter lowers a hot region's `&[Op]` to a WebAssembly module via the standard **dispatch loop** (a `loop` of per-basic-block `block`s + a `br_table` on a next-block local) so *any* control flow lowers, not just recognized shapes. The host is **cfg-split**: the pure-Rust `wasmi` interpreter natively (it doubles as the codegen oracle), and the platform's **real `WebAssembly`** (V8 in the browser / node) via `js_sys::WebAssembly` on wasm32 — the production tier. The synchronous `new WebAssembly.Module` / `new WebAssembly.Instance` constructors keep tier-up a plain step inside `Op::Call`; i64 crosses the JS boundary as `BigInt` (read back losslessly via base-10 string, no f64 round-trip).

**EXACT-integer arithmetic (in sync with every other tier).** Integer `add`/`sub`/`mul` are exact engine-wide — the VM promotes to BigInt on overflow (`semantics/arith.rs` `checked_add/sub/mul`) and the native forge tiers side-exit (`jo`) on overflow. The WASM-JIT matches: it emits the wrapping op then **traps (`unreachable`) on signed overflow** (add/sub via the sign-bit test, mul via an i128-free division check), so the trap propagates as a host error → `WasmTier::call` returns `None` → the task falls back to the VM, which promotes. **Never a silent wrap.** Div/Mod already trap on their edge cases (div-by-zero, `i64::MIN / -1`) → same fallback. Bitwise/shift (`BitXor`/`Shl`/`Shr`) lower to `i64.xor`/`i64.shl`/`i64.shr_s` — arithmetic shift with the count masked mod 64, matching the VM's `^`/`wrapping_shl`/`wrapping_shr` bit-for-bit. The differential's `check` enforces the contract both ways: a returned value equals the VM exactly; a trap means the VM genuinely overflowed (no spurious deopt).

**Verified:** `crates/logicaffeine_compile/tests/wasm_jit_browser.rs` — 5 `wasm-bindgen-test`s under node's real V8 (run by `scripts/test-wasm-node.sh` step 4/4): hand-built regions on the host, the i64↔BigInt boundary across `i64::MIN`/`i64::MAX`/`2^53+1`, a curated + seeded-fuzz differential vs the bytecode VM, and the real `WasmTier` tiering a hot function onto `WebAssembly`. `crates/logicaffeine_compile/tests/wasm_jit_differential.rs` runs the same differential natively (against `wasmi`) inside the fast suite's PASS 1b.

**Objective.** Give the browser a real JIT tier. The native x86 copy-and-patch JIT (`logicaffeine_forge`) cannot run in WASM — there is no executable memory to patch stencils into. The **only** way to reach JIT-level speed in the browser is a **second code-generator backend** that emits a **fresh WebAssembly module per hot function** and instantiates it via the host's `WebAssembly` constructors. This is a **different code generator** from the x86 stencil backend.

**Why separate.** Native keeps the copy-and-patch x86 JIT unchanged. The browser gets a parallel WASM-emitting backend behind the **same hot-detection / tiering policy** — they share the tier-up *decision* (when a function is hot enough to compile) but not the *backend* (x86 stencils vs emitted WASM bytes).

**Design.**
- **New backend** = a `wasm/` submodule (cargo feature) of `logicaffeine_forge`, gated to the `wasm32` target — the **same JIT-codegen concern** as the existing x86 backend, just a different target, so it lives in `forge`, **not a new crate**: a code generator from the VM's hot-region IR to a minimal WebAssembly module (one function + imports for runtime helpers). Distinct from `forge/x64asm.rs`.
- **Tier-up is async and fits the scheduler for free.** `WebAssembly.instantiate(bytes)` returns a Promise; in the cooperative driver this is just another **await point** — the task suspends until the module is ready, then resumes calling the instantiated export. No new concurrency machinery; it reuses the scheduler's yield/resume.
- **Per-tier-up cost.** Each hot function pays a one-time compile+instantiate latency, amortized over subsequent calls; the tiering threshold must be raised to only tier functions hot enough to amortize it.
- **Deopt-at-concurrency identical to Phase 6.** Concurrency ops stay JIT-ineligible; emitted modules are yield-free regions; a task hitting a concurrency op is back at the bytecode/tree-walker level.
- **Calling convention.** The emitted module imports the runtime's value-access / array / call helpers (the surface the VM already exposes); args/results marshalled as the VM's `Value` (with an i64/i32 fast path mirroring the native narrowing work).

**RED tests** (`crates/logicaffeine_tests/tests/wasm_jit.rs`, wasm-bindgen-test):
- `wasm_jit_hot_function_tiers_and_matches` — a hot pure-compute function gives identical output via the WASM-JIT tier and the tree-walker (differential, same seed).
- `wasm_jit_tierup_is_async_awaited` — tier-up suspends the task on instantiation and resumes correctly (assert via a drive-loop probe).
- `wasm_jit_body_with_chan_op_not_emitted` — a function containing a concurrency op is never WASM-JIT'd.
- `wasm_jit_speedup_smoke` — a hot loop is measurably faster than the tree-walker baseline (generous threshold).

**Definition of Done.** Behind a default-off feature; the default Studio build and all prior gates remain green; differential equivalence (WASM-JIT == tree-walker == VM) holds over the corpus × seed sweep.

---

## 6. Global regression gate (run after every phase)

A phase cannot advance until **all** of these pass:

1. **Benchmark byte-identity & timing** — every non-concurrent benchmark produces byte-identical output and timings within noise of baseline. The default AOT codegen for a representative benchmark equals its committed snapshot (`aot_default_codegen_for_benchmark_is_byte_identical`).
2. **Full suite** — `./scripts/run-all-tests-fast.sh` green (parity-proven; never two suites at once).
3. **Seeded reproducibility** — `run_with_seed(p,s)` identical across repeats (output + `SchedTrace`).
4. **Replay fidelity** — `run_with_trace(p, trace(p,s)) == run_with_seed(p,s).output`.
5. **Cross-tier differential** — `run_interpreter(seed) == run_vm(seed)` over the corpus (from Phase 5).
6. **Cross-driver equivalence** — `Cooperative(seed) == WorkStealing-serialized(seed)` (from Phase 7).
7. **No RED test modified.** **No two test suites run concurrently.**

---

## 7. Invariant ledger (must always hold)

- **I1 — Hot-path inviolate.** No non-concurrent program changes emission, bytecode, or tiering. (Gate #1.)
- **I2 — One choice point.** All nondeterminism flows through `Scheduler::decide`; nothing else draws entropy. (Audited in code review + replay tests.)
- **I3 — Isolated heaps.** No `Rc` crosses a task boundary; only `RtPayload` (Send) and `Arc` CRDT cells do. (Phase 4 analysis + `RtPayload: Send` compile assertion.)
- **I4 — Determinacy boundary is exactly §3.2's "FORCES" set.** Classifier, interpreter, AOT, TV agree. (Corpus determinacy table + cross-checks.)
- **I5 — Same semantics on both drivers** in serialized-decision mode. (Gate #6.)
- **I6 — Spec, not code, is shared with AOT.** The compiled binary never links `logicaffeine_runtime`. (Build/dependency check.)

---

## 8. Appendix — example programs (illustrative; confirm spellings against the corpus)

**A. Determinate producer/consumer**
```
Let ch be a Pipe of Int.
Launch a task to fill(ch).
Repeat 5 times:
    Receive x from ch.
    Show x.
```
Expected (any schedule, by Kahn): `1\n2\n3\n4\n5`.

**B. Nondeterminate select with timeout**
```
Let ch be a Pipe of Text.
Launch a task to slow_reply(ch).
Await the first of:
    Receive msg from ch:
        Show msg.
    After 1 seconds:
        Show "timed out".
```
Under a fixed `LOGOS_SEED`, the winner — hence output — is stable and identical interpreter↔compiled-Mode-B.

**C. CRDT shared counter synced across tasks/peers**
```
Let total be a ConvergentCount.
Launch a task to count_clicks(total).
Sync total on "clicks".
Show total.
```
Shared state is a CRDT cell (allowed by I3); `Sync` lowers to gossip (native libp2p / browser WS relay); received deltas `Merge` in.
