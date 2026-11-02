# logicaffeine-runtime

The deterministic concurrency runtime behind the LOGOS interpreter and VM: the task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed and trace that give LOGOS concurrency its operational semantics. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)`, so a race you hit once you can hit again, instead of chasing a heisenbug that never comes back. It is never linked into AOT-compiled binaries, which lower concurrency to host primitives through `logicaffeine-system`.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) workspace, tier 0. It has no dependencies at all: pure `std`, tokio-free, WASM-safe.

## Role in the workspace

The crate is a handful of focused modules: `scheduler` (the M:1 cooperative driver + `process_step` core), `executor` (the native M:N work-stealing driver), `task` (the `Task`/`TaskStep`/`TaskCtx` interface the execution tiers implement), `channel` (`ChanId` + the FIFO/rendezvous/unbounded channel semantics), `seed` (the `Chooser`/`SeededRng`/`SchedTrace` determinism contract), `payload` (`RtPayload`, the `Send` value that crosses task and OS-thread boundaries), and `config` (`SchedulerConfig`/`SchedulePolicy`/`ClockMode`).

This crate *is* the interpreter/VM concurrency model, not the AOT one; the compiled path uses `logicaffeine-system`'s platform services. Keeping the crate zero-dependency (empty `[dependencies]`, pure `std`) is what enforces that boundary at the dependency level. Marshalling between interpreter `RuntimeValue` / VM `Value` and the wire payload lives in `logicaffeine-compile`; this crate defines only the wire shape (`RtPayload`) and the scheduling engine. The user-facing, language-level concurrency story is in [`docs/concurrency.md`](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/concurrency.md).

**Determinism contract.** Every nondeterministic scheduling decision flows through one choke point, `Chooser::decide(kind, options) -> chosen`; nothing else draws entropy. In *record* mode it draws from a seeded SplitMix64 RNG (`SeededRng` — tiny, allocation-free, identical on native and WASM) and logs a `ChoicePoint { kind, options, chosen }`. In *replay* mode it re-issues the recorded choice and asserts the decision *shape* (kind + option count) still matches, panicking on divergence. The trace is id-agnostic (no `TaskId`/`ChanId`), so it never depends on identity allocation. `ChoiceKind` covers `TaskPick`, `SelectWinner`, `ChanWaiterWake`, `TimerTieBreak`, `WorkerPlacement`.

**Two drivers, one semantics.**

- **Cooperative (M:1)** — `Scheduler::run` multiplexes every `Task` on a single thread, resolving each decision through the embedded `Chooser`. This is the only driver under `wasm32`. A task is a small state machine: each `poll` advances it until it next blocks (channel / select / timer), yields, spawns, or exits; the scheduler parks it and resumes it through `TaskCtx`.
- **Work-stealing (M:N)** — `executor::run_workstealing_seeded` gives genuine multicore to the VM and the tree-walker using `std::thread::scope` + `std::sync::mpsc` (never tokio, never crossbeam). Because a task body holds `Rc` heaps and is irreducibly `!Send`, **tasks never cross threads**: only a `Send` `SpawnDesc` does, and each worker *builds* its `!Send` body locally from its own program clone. A central coordinator owns the single `Scheduler` + `Chooser`; workers only poll bodies in parallel and report a `Send` step back. The coordinator applies reports in deterministic pick order through the *same* `process_step`, so for a fixed seed the trace and output are byte-identical to the cooperative run. "Spawns are stealable; resumptions are pinned." This whole path is `#[cfg(not(target_arch = "wasm32"))]`, a target gate (no OS threads on wasm) rather than an opt-in Cargo feature.

Determinism holds under every `SchedulePolicy`: `Fifo`/`Lifo`/`RoundRobin`/`Priority` are deterministic by construction, and `Random` resolves its pick through the seeded `Chooser`.

## Public API

Re-exported from the crate root (see `src/lib.rs`):

```text
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
    pub fn run(&mut self) -> RunOutcome;                                  // poll_once to a fixpoint
    pub fn poll_once(&mut self) -> Option<RunOutcome>;                    // Some only at quiescence
    pub fn run_slice(&mut self, max_steps: usize) -> Option<RunOutcome>;  // the browser drive loop
    pub fn wake_io(&mut self) -> bool;                                    // re-ready BlockedIo tasks
    pub fn snapshot(&self) -> SchedSnapshot;
    pub fn into_trace(self) -> SchedTrace;
}

// Read-only observability: the Studio's Tasks/Channels strip, and deadlock narration.
pub struct SchedSnapshot { pub clock: u64, pub ready_len: usize, pub tasks: Vec<TaskView>, pub channels: Vec<ChanView> }
pub struct TaskView { pub id: TaskId, pub kind: TaskStateKind, pub is_main: bool }
pub struct ChanView { pub id: ChanId, pub depth: usize, pub capacity: Option<usize>,
                      pub blocked_senders: usize, pub blocked_receivers: usize, pub closed: bool }
pub enum TaskStateKind { Ready, BlockedRecv, BlockedSend, BlockedSelect, BlockedTimer, BlockedAwait, BlockedIo, Done }
pub fn narrate_deadlock(snap: &SchedSnapshot) -> String;      // scheduler::narrate_deadlock, not re-exported

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
    Await(TaskId), Abort(TaskId), Close(ChanId),
    IoPending,                                   // parked on host async I/O (network)
    Exit(RtPayload),
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
pub enum RunOutcome { Done(RtPayload), Deadlock, WaitingForIo }
pub struct ChanId(pub u64);
pub struct TaskId(pub u64);

// The Send wire value that crosses task (and OS-thread) boundaries.
pub enum RtPayload {
    Nothing, Int(i64), BigInt { .. }, Rational { .. }, Decimal { .. }, Money { .. }, Uuid(..),
    Complex { .. }, ComplexF { .. }, Modular { .. }, Quantity { .. },
    Float(f64), Bool, Char, Text(String), Word { .. },
    List(..), Tuple(..), Set(..), Map(..), Struct { .. }, Inductive { .. },
    Duration(i64), Date(i32), Moment(i64), Span { .. }, Time(i64),
    Chan(ChanId), TaskHandle(TaskId), Peer(String),
    Crdt(Vec<u8>),                               // a CRDT's full convergent state
}
```

Native-only (`#[cfg(not(target_arch = "wasm32"))]`), the work-stealing driver:

```text
pub fn run_workstealing_seeded<'env, B>(
    config: SchedulerConfig, seed: SchedSeed, main: SpawnDesc, build: B,
) -> WsOutcome
where B: Fn(SpawnDesc) -> Box<dyn Task<'env> + 'env> + Sync;

pub type FuncIdx = u16;
pub struct SpawnDesc { pub func: FuncIdx, pub args: Vec<RtPayload>, pub priority: u8, pub is_main: bool }
pub struct WsOutcome { pub outcome: RunOutcome, pub output: Vec<String>, pub trace: SchedTrace }
```

The interpreter and VM implement `Task` over their own continuations; `build` reconstructs a `!Send` body from a `Send` `SpawnDesc` per worker. Under `ClockMode::Logical` the timer wheel runs on a virtual clock, so `Sleep`/`After` are ordered logically and elapse instantly; same-tick timers are tie-broken through the seeded `Chooser`, keeping timeout races replayable. A closed channel delivers `Nothing` on receive instead of blocking and counts as receive-ready for `Select`. A woken `Select` withdraws every registration it made, including a second arm on the same channel. A `Crdt` payload carries the replica's full state rather than its shared handle, so the receiver builds a fresh replica and a redelivered payload never double-counts.

When the scheduler proves a deadlock, `narrate_deadlock` renders the snapshot as prose: each blocked task with what it waits on, and each channel's fill, blocked senders and receivers, and closed state. `RunOutcome::WaitingForIo` is not a deadlock: a task is parked on network I/O the scheduler has no reactor for, and the async drive loop yields to the host reactor (tokio natively, the browser event loop on wasm) and then calls `wake_io`.

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `cooperative` | on | Names the M:1 cooperative driver (single-thread and event-loop, the only driver under WASM); every build compiles it. |

There is **no `work_stealing` feature**. The native M:N work-stealing driver (`executor`) is compiled automatically off-WASM via `#[cfg(not(target_arch = "wasm32"))]` and excluded on `wasm32` (no OS threads). Genuine multicore is a property of the native build, not an opt-in toggle.

## Dependencies

None. `[dependencies]` is empty and the crate uses only `std`, which is what keeps it WASM-safe and tokio-free.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/crates/logicaffeine_runtime/CHANGELOG.md)
