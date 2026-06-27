//! Tasks — the unit the scheduler multiplexes.
//!
//! A task is a small state machine: each `poll` advances it until it next wants
//! to block (on a channel, a select, or a timer), yield, spawn, or exit. The
//! scheduler drives `poll`, parks the task on blocking steps, and resumes it
//! (delivering any value) when the block clears. The interpreter and VM each
//! implement [`Task`] over their own continuation; the toy tasks in the tests
//! implement it by hand.

use crate::channel::ChanId;
use crate::payload::RtPayload;

/// A scheduler-assigned task handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub u64);

/// One arm of a `Select` (`Await the first of:`).
#[derive(Debug, Clone)]
pub enum SelectArm {
    /// `Receive x from ch:` — ready when `ch` has a value (or a waiting sender).
    Recv(ChanId),
    /// `After d ticks:` — fires after `d` logical ticks if no earlier arm wins.
    Timeout(u64),
}

/// What a task asks the scheduler to do after a `poll`.
pub enum TaskStep<'t> {
    /// Cooperative yield — re-enqueue me behind the other ready tasks.
    Yield,
    /// Block until a value can be received from `ch`; resume with that value
    /// (delivered as `TaskCtx::resumed_with`).
    Recv(ChanId),
    /// Block until `ch` has room, then hand it `payload`; resume.
    Send(ChanId, RtPayload),
    /// Non-blocking send: hand `payload` to `ch` if it can be taken right now
    /// (a waiting receiver or room in the buffer); resume immediately with
    /// `RtPayload::Bool(true)` on success, `Bool(false)` if it would have blocked.
    TrySend(ChanId, RtPayload),
    /// Non-blocking receive: resume immediately with a value if one is available
    /// (in `TaskCtx::resumed_with`), or `RtPayload::Nothing` if the channel is empty.
    TryRecv(ChanId),
    /// Block on a choice over several arms; resume with the winning arm's index
    /// in `TaskCtx::selected_arm` (and its value in `resumed_with` for a recv arm).
    Select(Vec<SelectArm>),
    /// Block for `ticks` logical ticks.
    Sleep(u64),
    /// Create a channel with the given capacity; resume immediately with its
    /// `ChanId` (delivered in `TaskCtx::new_chan`).
    NewChan(Option<usize>),
    /// Spawn a child task; resume immediately with its `TaskId` (in `TaskCtx::spawned`).
    Spawn(Box<dyn Task<'t> + 't>),
    /// Spawn a child *by descriptor* (function index + already-materialised
    /// arguments) rather than a boxed body — resume with its `TaskId`. The boxed
    /// [`TaskStep::Spawn`] cannot cross an OS-thread boundary (the body is `!Send`);
    /// the work-stealing driver uses this `Send` form so any worker can build the
    /// child locally from the descriptor. (The cooperative driver builds inline and
    /// keeps using `Spawn`.)
    SpawnDesc { func: u16, args: Vec<RtPayload>, want_handle: bool },
    /// Block until task `TaskId` finishes; resume with its result payload
    /// (`Nothing` if that task was aborted). Backs awaiting a task handle.
    Await(TaskId),
    /// Cancel task `TaskId` (backs `Stop handle`); resume immediately. The
    /// aborted task never polls again and its awaiters observe an aborted result.
    Abort(TaskId),
    /// Close channel `ChanId`; resume immediately. Subsequent receives on the
    /// drained channel return `Nothing` rather than blocking.
    Close(ChanId),
    /// Blocked on external async I/O (a network `await` — `Connect`/`Listen`/peer
    /// messaging) that the scheduler itself cannot service: it has no reactor. The task is
    /// parked `BlockedIo` and re-polled by the async drive loop after it yields to the host
    /// reactor (tokio natively, the browser event loop on wasm). A purely channel/timer
    /// program never produces this, so its behavior is byte-identical.
    IoPending,
    /// Finished, with a result payload.
    Exit(RtPayload),
}

/// Per-`poll` context: what the task was resumed with.
pub struct TaskCtx {
    /// The value delivered by the blocking step that just resumed this task
    /// (the received value for `Recv`/winning `Select` recv arm; `Nothing` otherwise).
    pub resumed_with: RtPayload,
    /// For a resumed `Select`, the index of the arm that won; `None` otherwise.
    pub selected_arm: Option<usize>,
    /// For a resumed `NewChan`, the id of the freshly-created channel.
    pub new_chan: Option<ChanId>,
    /// For a resumed `Spawn`, the id of the spawned task (its handle).
    pub spawned: Option<TaskId>,
}

/// A schedulable unit of work. `'t` ties any spawned children to the lifetime of
/// borrowed data the task closes over (e.g. the interpreter's borrowed AST).
pub trait Task<'t> {
    /// Advance the task until its next scheduling point.
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'t>;

    /// Scheduling priority — higher runs first under [`crate::SchedulePolicy::Priority`].
    /// Defaults to 0 (all tasks equal).
    fn priority(&self) -> u8 {
        0
    }

    /// Output lines the task produced during the *just-finished* `poll` slice, in
    /// order. The work-stealing driver collects these per slice and flushes them in
    /// the deterministic pick-order apply, so concurrent output matches the
    /// cooperative interleaving byte-for-byte. Cooperative tasks write straight to
    /// their sink and leave this empty (the default).
    fn take_output(&mut self) -> Vec<String> {
        Vec::new()
    }
}
