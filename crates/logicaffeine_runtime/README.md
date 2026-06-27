# logicaffeine-runtime

The deterministic, replayable concurrency runtime for the Logicaffeine interpreter and VM: the task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery that give Logos concurrency its operational semantics. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)`.

Part of the [Logicaffeine](../../NEW_README.md) workspace. Tier 0 — deliberately dependency-free (pure std, tokio-free, WASM-safe). Used by the interpreter/VM tiers; NOT linked into AOT binaries (which lower to host primitives via `logicaffeine_system`).

## Role in the workspace

This crate *is* the interpreter/VM concurrency model, not the AOT one. By charter it is never linked into AOT-compiled binaries (invariant I6 of `FINISH_INTERPRETER.md`) — the compiled path uses `logicaffeine-system`'s platform services. Keeping the crate zero-dependency (empty `[dependencies]`, pure `std`) is what enforces that boundary at the dependency level. Marshalling between interpreter `RuntimeValue` / VM `Value` and the wire payload lives in `logicaffeine-compile`; this crate defines only the wire shape (`RtPayload`) and the scheduling engine. The user-facing, language-level concurrency story is in [`new_docs/concurrency.md`](../../new_docs/concurrency.md).

**Determinism contract.** Every nondeterministic scheduling decision flows through one choke point, `Chooser::decide(kind, options) -> chosen`; nothing else draws entropy. In *record* mode it draws from a seeded SplitMix64 RNG (`SeededRng` — tiny, allocation-free, identical on native and WASM) and logs a `ChoicePoint { kind, options, chosen }`; in *replay* mode it re-issues the recorded choice and asserts the decision *shape* (kind + option count) still matches, panicking on divergence. The trace is id-agnostic (no `TaskId`/`ChanId`), so it never depends on identity allocation. `ChoiceKind` covers `TaskPick`, `SelectWinner`, `ChanWaiterWake`, `TimerTieBreak`, `WorkerPlacement`.

**Two drivers, one semantics.**

- **Cooperative (M:1)** — `Scheduler::run` multiplexes every `Task` on a single thread, resolving each decision through the embedded `Chooser`. This is the only driver under `wasm32`. A task is a small state machine: each `poll` advances it until it next blocks (channel / select / timer), yields, spawns, or exits; the scheduler parks it and resumes it through `TaskCtx`.
- **Work-stealing (M:N)** — `executor::run_workstealing_seeded` gives genuine multicore to the VM/tree-walker tiers using `std::thread::scope` + `std::sync::mpsc` (never tokio, never crossbeam). Because a task body holds `Rc` heaps and is irreducibly `!Send`, **tasks never cross threads** — only a `Send` `SpawnDesc` does, and each worker *builds* its `!Send` body locally from its own program clone. A central coordinator owns the single `Scheduler` + `Chooser`; workers only poll bodies in parallel and report a `Send` step back. The coordinator applies reports in deterministic pick order through the *same* `process_step`, so for a fixed seed the trace and output are byte-identical to the cooperative run. "Spawns are stealable; resumptions are pinned." This whole path is `#[cfg(not(target_arch = "wasm32"))]` — a target gate (no OS threads on wasm), not an opt-in Cargo feature.

Determinism holds under every `SchedulePolicy`: `Fifo`/`Lifo`/`RoundRobin`/`Priority` are deterministic by construction, and `Random` resolves its pick through the seeded `Chooser`.

## Public API

Re-exported from the crate root (see `src/lib.rs`):

```rust
// Entry points — record once, replay forever.
pub fn run_with_seed<'t, F: FnOnce(&mut Scheduler<'t>)>(
    config: SchedulerConfig, seed: SchedSeed, setup: F,
) -> (RunOutcome, SchedTrace);
pub fn run_with_trace<'t, F: FnOnce(&mut Scheduler<'t>)>(
    config: SchedulerConfig, trace: SchedTrace, setup: F,
) -> RunOutcome;

// The scheduler.
impl<'t> Scheduler<'t> {
    pub fn new(config: SchedulerConfig, chooser: Chooser) -> Self;
    pub fn spawn(&mut self, task: Box<dyn Task<'t> + 't>) -> TaskId;
    pub fn spawn_main(&mut self, task: Box<dyn Task<'t> + 't>) -> TaskId; // its Exit ⇒ Done payload
    pub fn new_chan(&mut self, capacity: Option<usize>) -> ChanId;       // None=unbounded, Some(0)=rendezvous
    pub fn new_default_chan(&mut self) -> ChanId;
    pub fn run(&mut self) -> RunOutcome;                                  // Done(payload) | Deadlock
    pub fn into_trace(self) -> SchedTrace;
}

// The unit the scheduler multiplexes.
pub trait Task<'t> {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t>;
    fn priority(&self) -> u8 { 0 }
    fn take_output(&mut self) -> Vec<String> { Vec::new() } // collected per slice for M:N pick-order flush
}

// What a poll asks the scheduler to do next.
pub enum TaskStep<'t> {
    Yield, Recv(ChanId), Send(ChanId, RtPayload),
    TrySend(ChanId, RtPayload), TryRecv(ChanId),
    Select(Vec<SelectArm>), Sleep(u64), NewChan(Option<usize>),
    Spawn(Box<dyn Task<'t> + 't>),
    SpawnDesc { func: u16, args: Vec<RtPayload>, want_handle: bool }, // Send form for M:N
    Await(TaskId), Abort(TaskId), Close(ChanId), Exit(RtPayload),
}
pub enum SelectArm { Recv(ChanId), Timeout(u64) }       // u64 = logical ticks
pub struct TaskCtx { resumed_with, selected_arm, new_chan, spawned }

// The determinism contract.
pub enum Chooser { Record { .. }, Replay { .. } }
impl Chooser {
    pub fn record(seed: SchedSeed) -> Self;
    pub fn replay(trace: SchedTrace) -> Self;
    pub fn decide(&mut self, kind: ChoiceKind, options: usize) -> usize;
    pub fn into_trace(self) -> SchedTrace;
}
pub struct SchedSeed(pub u64);
pub struct SchedTrace { pub seed: SchedSeed, pub choices: Vec<ChoicePoint> }
pub struct ChoicePoint { pub kind: ChoiceKind, pub options: usize, pub chosen: usize }
pub enum ChoiceKind { TaskPick, SelectWinner, ChanWaiterWake, TimerTieBreak, WorkerPlacement }
pub struct SeededRng { /* SplitMix64 */ }

// Config (fluent setters: with_policy / with_clock / with_channel_capacity / with_workers).
pub struct SchedulerConfig { policy, clock, default_channel_capacity: 32, preempt_every: 10_000, workers: 1 }
pub enum SchedulePolicy { Fifo /*default*/, Lifo, RoundRobin, Random, Priority }
pub enum ClockMode { Logical /*default, virtual time*/, Wall }
pub enum RunOutcome { Done(RtPayload), Deadlock }
pub struct ChanId(pub u64);
pub struct TaskId(pub u64);

// The Send wire value that crosses task (and OS-thread) boundaries.
pub enum RtPayload {
    Nothing, Int(i64), BigInt { .. }, Rational { .. }, Float(f64), Bool, Char, Text(String),
    List(..), Tuple(..), Set(..), Map(..), Struct { .. }, Inductive { .. },
    Duration(i64), Date(i32), Moment(i64), Span { .. }, Time(i64),
    Chan(ChanId), TaskHandle(TaskId), Peer(String),
}
```

Native-only (`#[cfg(not(target_arch = "wasm32"))]`), the work-stealing driver:

```rust
pub fn run_workstealing_seeded<'env, B>(
    config: SchedulerConfig, seed: SchedSeed, main: SpawnDesc, build: B,
) -> WsOutcome
where B: Fn(SpawnDesc) -> Box<dyn Task<'env> + 'env> + Sync;

pub type FuncIdx = u16;
pub struct SpawnDesc { pub func: FuncIdx, pub args: Vec<RtPayload>, pub priority: u8, pub is_main: bool }
pub struct WsOutcome { pub outcome: RunOutcome, pub output: Vec<String>, pub trace: SchedTrace }
```

The interpreter and VM implement `Task` over their own continuations; `build` reconstructs a `!Send` body from a `Send` `SpawnDesc` per worker. Under `ClockMode::Logical` the timer wheel runs on a virtual clock, so `Sleep`/`After` are ordered logically and elapse instantly; same-tick timers are tie-broken through the seeded `Chooser`, keeping timeout races replayable. A closed channel delivers `Nothing` on receive instead of blocking and counts as receive-ready for `Select`.

## Feature flags

| Feature | Default | Gates |
|---------|---------|-------|
| `cooperative` | yes | The M:1 cooperative single-thread/event-loop driver — the only mode under WASM. (Currently a no-op marker: the cooperative scheduler is always compiled.) |

There is **no `work_stealing` feature**. The native M:N work-stealing driver (`executor`) is compiled automatically off-WASM via `#[cfg(not(target_arch = "wasm32"))]` and excluded on `wasm32` (no OS threads). Genuine multicore is a property of the native build, not an opt-in toggle.

## License

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)
