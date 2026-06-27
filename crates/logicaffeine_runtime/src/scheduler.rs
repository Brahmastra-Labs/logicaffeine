//! The scheduler core + cooperative driver.
//!
//! A single, deterministic, seed-driven scheduler that multiplexes [`Task`]s over
//! channels, `Select`, and timers. The cooperative driver ([`Scheduler::run`])
//! runs everything on one thread, resolving every nondeterministic decision
//! through the embedded [`Chooser`] (so a fixed seed ⇒ a fixed run, and a trace
//! ⇒ an exact replay). The ready-selection discipline is the configured
//! [`SchedulePolicy`]; determinism holds under all of them.

use std::collections::HashMap;

use crate::channel::{Chan, ChanId};
use crate::config::{SchedulePolicy, SchedulerConfig};
use crate::payload::RtPayload;
use crate::seed::{ChoiceKind, Chooser, SchedSeed, SchedTrace};
use crate::task::{SelectArm, Task, TaskCtx, TaskId, TaskStep};

/// The result of running a scheduler to quiescence.
#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    /// Every task finished. Carries the main task's result (or `Nothing`).
    Done(RtPayload),
    /// Tasks remain but all are blocked with no timer to fire — a deadlock.
    Deadlock,
    /// Tasks remain, none ready and no timer to fire, but at least one is parked on
    /// external async I/O ([`TaskState::BlockedIo`]) — not a deadlock. The scheduler has no
    /// reactor; the async drive loop must yield to the host reactor, then [`Scheduler::wake_io`]
    /// to re-poll the parked tasks. The synchronous `run()` cannot make progress here.
    WaitingForIo,
}

enum TaskState {
    Ready,
    BlockedRecv,
    BlockedSend,
    BlockedSelect,
    BlockedTimer,
    BlockedAwait,
    BlockedIo,
    Done,
}

/// A public, read-only projection of a task's scheduling state — what the Studio's
/// Tasks/Channels strip renders. Mirrors the private [`TaskState`] without exposing the
/// task body or any mutable handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStateKind {
    Ready,
    BlockedRecv,
    BlockedSend,
    BlockedSelect,
    BlockedTimer,
    BlockedAwait,
    BlockedIo,
    Done,
}

impl TaskState {
    fn kind(&self) -> TaskStateKind {
        match self {
            TaskState::Ready => TaskStateKind::Ready,
            TaskState::BlockedRecv => TaskStateKind::BlockedRecv,
            TaskState::BlockedSend => TaskStateKind::BlockedSend,
            TaskState::BlockedSelect => TaskStateKind::BlockedSelect,
            TaskState::BlockedTimer => TaskStateKind::BlockedTimer,
            TaskState::BlockedAwait => TaskStateKind::BlockedAwait,
            TaskState::BlockedIo => TaskStateKind::BlockedIo,
            TaskState::Done => TaskStateKind::Done,
        }
    }
}

/// A read-only view of one task for [`Scheduler::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskView {
    pub id: TaskId,
    pub kind: TaskStateKind,
    pub is_main: bool,
}

/// A read-only view of one channel for [`Scheduler::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChanView {
    pub id: ChanId,
    /// Buffered values waiting to be received.
    pub depth: usize,
    /// `None` for an unbounded channel, `Some(0)` for a rendezvous channel.
    pub capacity: Option<usize>,
    pub blocked_senders: usize,
    pub blocked_receivers: usize,
    pub closed: bool,
}

/// A deterministic, read-only snapshot of the scheduler between steps — the observability
/// seam the browser drive loop emits to the Studio so a running concurrent program shows
/// its live task and channel state instead of freezing the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedSnapshot {
    /// The logical clock (the timer wheel's current tick).
    pub clock: u64,
    /// How many tasks are ready to run right now.
    pub ready_len: usize,
    /// Every task, sorted by id (deterministic order for a stable UI).
    pub tasks: Vec<TaskView>,
    /// Every channel, sorted by id.
    pub channels: Vec<ChanView>,
}

/// How a finished task finished — delivered to anyone awaiting it.
#[derive(Debug, Clone)]
enum AwaitResult {
    Completed(RtPayload),
    Aborted,
}

impl AwaitResult {
    fn payload(&self) -> RtPayload {
        match self {
            AwaitResult::Completed(p) => p.clone(),
            AwaitResult::Aborted => RtPayload::Nothing,
        }
    }
}

struct TaskSlot<'t> {
    task: Option<Box<dyn Task<'t> + 't>>,
    priority: u8,
    resume: RtPayload,
    selected_arm: Option<usize>,
    select_arms: Option<Vec<SelectArm>>,
    /// Channel id to deliver on the next poll (after a `NewChan`).
    resume_chan: Option<ChanId>,
    /// Task id to deliver on the next poll (after a `Spawn`).
    resume_task: Option<TaskId>,
    state: TaskState,
    is_main: bool,
}

struct TimerEntry {
    fire_at: u64,
    tid: TaskId,
    /// `Some(i)` if this is the timeout arm `i` of a `Select`; `None` for a plain `Sleep`.
    arm: Option<usize>,
}

/// The resume state of a ready task, taken by the work-stealing coordinator
/// ([`crate::executor`]) to deliver to a worker thread. Mirrors the `TaskCtx` the
/// cooperative `run` builds inline. (Native-only.)
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct WsDispatch {
    pub tid: TaskId,
    pub priority: u8,
    pub selected_arm: Option<usize>,
    pub resume: RtPayload,
    pub new_chan: Option<ChanId>,
    pub spawned: Option<TaskId>,
}

/// The deterministic task scheduler.
pub struct Scheduler<'t> {
    config: SchedulerConfig,
    chooser: Chooser,
    tasks: HashMap<TaskId, TaskSlot<'t>>,
    ready: Vec<TaskId>,
    chans: HashMap<ChanId, Chan>,
    timers: Vec<TimerEntry>,
    clock: u64,
    next_id: u64,
    main_result: Option<RtPayload>,
    results: HashMap<TaskId, AwaitResult>,
    awaiters: HashMap<TaskId, Vec<TaskId>>,
}

impl<'t> Scheduler<'t> {
    /// A fresh scheduler with the given config and decision source.
    pub fn new(config: SchedulerConfig, chooser: Chooser) -> Self {
        Scheduler {
            config,
            chooser,
            tasks: HashMap::new(),
            ready: Vec::new(),
            chans: HashMap::new(),
            timers: Vec::new(),
            clock: 0,
            next_id: 0,
            main_result: None,
            results: HashMap::new(),
            awaiters: HashMap::new(),
        }
    }

    fn bump(&mut self) -> u64 {
        let v = self.next_id;
        self.next_id += 1;
        v
    }

    /// Create a channel with an explicit capacity (`None` = unbounded, `Some(0)` = rendezvous).
    pub fn new_chan(&mut self, capacity: Option<usize>) -> ChanId {
        let id = ChanId(self.bump());
        self.chans.insert(id, Chan::new(capacity));
        id
    }

    /// Create a channel with the config's default capacity.
    pub fn new_default_chan(&mut self) -> ChanId {
        let cap = self.config.default_channel_capacity;
        self.new_chan(Some(cap))
    }

    /// Spawn a task. Returns its id; it is enqueued ready.
    pub fn spawn(&mut self, task: Box<dyn Task<'t> + 't>) -> TaskId {
        let priority = task.priority();
        self.spawn_inner(Some(task), priority, false)
    }

    /// Spawn the *main* task — its `Exit` value becomes the run's [`RunOutcome::Done`] payload.
    pub fn spawn_main(&mut self, task: Box<dyn Task<'t> + 't>) -> TaskId {
        let priority = task.priority();
        self.spawn_inner(Some(task), priority, true)
    }

    fn spawn_inner(
        &mut self,
        task: Option<Box<dyn Task<'t> + 't>>,
        priority: u8,
        is_main: bool,
    ) -> TaskId {
        let id = TaskId(self.bump());
        self.tasks.insert(
            id,
            TaskSlot {
                task,
                priority,
                resume: RtPayload::Nothing,
                selected_arm: None,
                select_arms: None,
                resume_chan: None,
                resume_task: None,
                state: TaskState::Ready,
                is_main,
            },
        );
        self.ready.push(id);
        id
    }

    /// Spawn a *metadata-only* slot (no task body) for the work-stealing driver:
    /// the body lives worker-side, built from a `Send` descriptor. Otherwise
    /// identical to [`Self::spawn`] — enqueued ready, same id allocation. Returns
    /// its id. (Native-only: the coordinator that uses it is `cfg(not(wasm32))`.)
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn spawn_meta(&mut self, priority: u8, is_main: bool) -> TaskId {
        self.spawn_inner(None, priority, is_main)
    }

    /// Consume the scheduler and return the recorded scheduling trace.
    pub fn into_trace(self) -> SchedTrace {
        self.chooser.into_trace()
    }

    /// Run cooperatively to quiescence.
    pub fn run(&mut self) -> RunOutcome {
        loop {
            if let Some(outcome) = self.poll_once() {
                return outcome;
            }
        }
    }

    /// Advance the scheduler by a single step: dispatch one ready task, or (if none is
    /// ready) advance the timer wheel. Returns `Some(outcome)` only at quiescence — when no
    /// task is ready and no timer can fire — and `None` while there is still work to do.
    /// `run` is `poll_once` to a fixpoint; the browser drive loop calls it in slices,
    /// yielding a macrotask between them so the UI repaints. Both produce identical
    /// behavior and identical traces by construction.
    pub fn poll_once(&mut self) -> Option<RunOutcome> {
        if let Some(tid) = self.pick_ready() {
            let step = {
                let slot = self.tasks.get_mut(&tid).expect("ready task exists");
                let mut boxed = slot.task.take().expect("task body present");
                let mut ctx = TaskCtx {
                    resumed_with: std::mem::replace(&mut slot.resume, RtPayload::Nothing),
                    selected_arm: slot.selected_arm.take(),
                    new_chan: slot.resume_chan.take(),
                    spawned: slot.resume_task.take(),
                };
                let step = boxed.poll(&mut ctx);
                slot.task = Some(boxed);
                step
            };
            self.process_step(tid, step);
            None
        } else if !self.timers.is_empty() {
            self.advance_timers();
            None
        } else {
            let all_done = self.tasks.values().all(|s| matches!(s.state, TaskState::Done));
            let any_io = self
                .tasks
                .values()
                .any(|s| matches!(s.state, TaskState::BlockedIo));
            Some(if all_done {
                RunOutcome::Done(self.main_result.clone().unwrap_or(RtPayload::Nothing))
            } else if any_io {
                // Not a deadlock: a task awaits external I/O the async drive loop will service.
                RunOutcome::WaitingForIo
            } else {
                RunOutcome::Deadlock
            })
        }
    }

    /// Run at most `max_steps` steps. Returns `Some(outcome)` if quiescence was reached
    /// within the budget, or `None` if work remains (the caller should yield and call
    /// again). A `max_steps` of 0 makes no progress and returns `None`.
    pub fn run_slice(&mut self, max_steps: usize) -> Option<RunOutcome> {
        for _ in 0..max_steps {
            if let Some(outcome) = self.poll_once() {
                return Some(outcome);
            }
        }
        None
    }

    /// Re-ready every task parked on external I/O ([`TaskState::BlockedIo`]), in ascending
    /// id order (deterministic). The async drive loop calls this after yielding to the host
    /// reactor so the parked tasks re-poll their network futures. Returns whether any task was
    /// woken. Channel/timer tasks are untouched, so a non-networking program never sees this.
    pub fn wake_io(&mut self) -> bool {
        let mut io_tasks: Vec<TaskId> = self
            .tasks
            .iter()
            .filter(|(_, s)| matches!(s.state, TaskState::BlockedIo))
            .map(|(id, _)| *id)
            .collect();
        io_tasks.sort_by_key(|t| t.0);
        for tid in &io_tasks {
            let slot = self.tasks.get_mut(tid).unwrap();
            slot.state = TaskState::Ready;
            self.ready.push(*tid);
        }
        !io_tasks.is_empty()
    }

    /// A deterministic, read-only snapshot of the current task and channel state — emitted
    /// between slices to drive the Studio's Tasks/Channels strip. Tasks and channels are
    /// sorted by id so the UI order is stable across snapshots.
    pub fn snapshot(&self) -> SchedSnapshot {
        let mut tasks: Vec<TaskView> = self
            .tasks
            .iter()
            .map(|(id, slot)| TaskView { id: *id, kind: slot.state.kind(), is_main: slot.is_main })
            .collect();
        tasks.sort_by_key(|t| t.id.0);
        let mut channels: Vec<ChanView> = self
            .chans
            .iter()
            .map(|(id, ch)| ChanView {
                id: *id,
                depth: ch.queue.len(),
                capacity: ch.capacity,
                blocked_senders: ch.blocked_senders.len(),
                blocked_receivers: ch.blocked_receivers.len(),
                closed: ch.closed,
            })
            .collect();
        channels.sort_by_key(|c| c.id.0);
        SchedSnapshot { clock: self.clock, ready_len: self.ready.len(), tasks, channels }
    }

    // ─── Work-stealing coordinator hooks (native-only) ──────────────────────
    // The work-stealing driver (`executor.rs`) owns a `Scheduler` with
    // metadata-only slots and drives it externally: it dispatches ready tasks to
    // worker threads (each builds + polls the `!Send` body locally) and applies
    // the workers' `Send` `StepReport`s through the SAME `process_step`/`do_*`
    // methods — so channel and decision semantics, and the seeded trace, are
    // identical to the cooperative run.

    /// Spawn a metadata child + ready the parent with the child's handle, exactly
    /// as the cooperative `Spawn` step does. Returns the child's id (the
    /// coordinator records its build descriptor under that id).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_spawn_child(&mut self, parent: TaskId, priority: u8) -> TaskId {
        let child = self.spawn_meta(priority, false);
        self.tasks.get_mut(&parent).unwrap().resume_task = Some(child);
        self.mark_ready(parent, RtPayload::Nothing);
        child
    }

    /// Pick the next ready task (the same `pick_ready` discipline as cooperative)
    /// and take its resume state, for the coordinator to deliver to a worker.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_next_dispatch(&mut self) -> Option<WsDispatch> {
        let tid = self.pick_ready()?;
        let slot = self.tasks.get_mut(&tid).expect("ready task exists");
        Some(WsDispatch {
            tid,
            priority: slot.priority,
            selected_arm: slot.selected_arm.take(),
            resume: std::mem::replace(&mut slot.resume, RtPayload::Nothing),
            new_chan: slot.resume_chan.take(),
            spawned: slot.resume_task.take(),
        })
    }

    /// Every task finished?
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_all_done(&self) -> bool {
        self.tasks.values().all(|s| matches!(s.state, TaskState::Done))
    }

    /// Are there pending timers (advance the logical clock when no task is ready)?
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_has_timers(&self) -> bool {
        !self.timers.is_empty()
    }

    /// Advance the timer wheel (marks timer / select-timeout tasks ready).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_advance_timers(&mut self) {
        self.advance_timers();
    }

    /// The main task's result (or `Nothing`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ws_main_result(&self) -> RtPayload {
        self.main_result.clone().unwrap_or(RtPayload::Nothing)
    }

    fn pick_ready(&mut self) -> Option<TaskId> {
        if self.ready.is_empty() {
            return None;
        }
        let idx = match self.config.policy {
            SchedulePolicy::Fifo | SchedulePolicy::RoundRobin => 0,
            SchedulePolicy::Lifo => self.ready.len() - 1,
            SchedulePolicy::Random => self.chooser.decide(ChoiceKind::TaskPick, self.ready.len()),
            SchedulePolicy::Priority => {
                let mut best = 0usize;
                let mut best_pri = self.tasks[&self.ready[0]].priority;
                for i in 1..self.ready.len() {
                    let p = self.tasks[&self.ready[i]].priority;
                    if p > best_pri {
                        best_pri = p;
                        best = i;
                    }
                }
                best
            }
        };
        Some(self.ready.remove(idx))
    }

    pub(crate) fn process_step(&mut self, tid: TaskId, step: TaskStep<'t>) {
        match step {
            TaskStep::Yield => self.mark_ready(tid, RtPayload::Nothing),
            TaskStep::Await(target) => self.do_await(tid, target),
            TaskStep::Abort(target) => {
                self.do_abort(target);
                self.mark_ready(tid, RtPayload::Nothing);
            }
            TaskStep::Close(ch) => {
                self.do_close(ch);
                self.mark_ready(tid, RtPayload::Nothing);
            }
            TaskStep::Exit(v) => {
                if self.tasks[&tid].is_main {
                    self.main_result = Some(v.clone());
                }
                self.tasks.get_mut(&tid).unwrap().state = TaskState::Done;
                self.complete_task(tid, AwaitResult::Completed(v));
            }
            TaskStep::Spawn(child) => {
                let child_id = self.spawn(child);
                self.tasks.get_mut(&tid).unwrap().resume_task = Some(child_id);
                self.mark_ready(tid, RtPayload::Nothing);
            }
            // The descriptor form is produced only by work-stealing tasks and is
            // serviced by the coordinator (which builds the child worker-side),
            // never the inline `process_step`.
            TaskStep::SpawnDesc { .. } => {
                unreachable!("SpawnDesc is serviced by the work-stealing coordinator, not process_step")
            }
            TaskStep::NewChan(cap) => {
                let capacity = Some(cap.unwrap_or(self.config.default_channel_capacity));
                let id = self.new_chan(capacity);
                self.tasks.get_mut(&tid).unwrap().resume_chan = Some(id);
                self.mark_ready(tid, RtPayload::Nothing);
            }
            TaskStep::Send(ch, payload) => self.do_send(tid, ch, payload),
            TaskStep::TrySend(ch, payload) => self.do_try_send(tid, ch, payload),
            TaskStep::Recv(ch) => self.do_recv(tid, ch),
            TaskStep::TryRecv(ch) => self.do_try_recv(tid, ch),
            TaskStep::Sleep(d) => {
                self.timers.push(TimerEntry { fire_at: self.clock + d, tid, arm: None });
                self.tasks.get_mut(&tid).unwrap().state = TaskState::BlockedTimer;
            }
            TaskStep::Select(arms) => self.do_select(tid, arms),
            TaskStep::IoPending => {
                // Parked on external async I/O. The scheduler has no reactor; the async drive
                // loop re-readies this task via `wake_io` after yielding to the host reactor.
                self.tasks.get_mut(&tid).unwrap().state = TaskState::BlockedIo;
            }
        }
    }

    fn mark_ready(&mut self, tid: TaskId, value: RtPayload) {
        let slot = self.tasks.get_mut(&tid).unwrap();
        slot.resume = value;
        slot.state = TaskState::Ready;
        self.ready.push(tid);
    }

    fn do_send(&mut self, tid: TaskId, ch: ChanId, payload: RtPayload) {
        let waiting_receiver = self.chans.get_mut(&ch).unwrap().blocked_receivers.pop_front();
        if let Some(rid) = waiting_receiver {
            self.deliver_to_receiver(rid, ch, payload);
            self.mark_ready(tid, RtPayload::Nothing);
            return;
        }
        if self.chans[&ch].has_room() {
            self.chans.get_mut(&ch).unwrap().queue.push_back(payload);
            self.mark_ready(tid, RtPayload::Nothing);
        } else {
            self.chans.get_mut(&ch).unwrap().blocked_senders.push_back((tid, payload));
            self.tasks.get_mut(&tid).unwrap().state = TaskState::BlockedSend;
        }
    }

    fn do_recv(&mut self, tid: TaskId, ch: ChanId) {
        let from_queue = self.chans.get_mut(&ch).unwrap().queue.pop_front();
        if let Some(v) = from_queue {
            self.mark_ready(tid, v);
            self.refill_from_blocked_sender(ch);
            return;
        }
        let from_sender = self.chans.get_mut(&ch).unwrap().blocked_senders.pop_front();
        if let Some((sid, payload)) = from_sender {
            self.mark_ready(tid, payload);
            self.mark_ready(sid, RtPayload::Nothing);
            return;
        }
        if self.chans[&ch].closed {
            self.mark_ready(tid, RtPayload::Nothing);
            return;
        }
        self.chans.get_mut(&ch).unwrap().blocked_receivers.push_back(tid);
        self.tasks.get_mut(&tid).unwrap().state = TaskState::BlockedRecv;
    }

    fn do_try_send(&mut self, tid: TaskId, ch: ChanId, payload: RtPayload) {
        let waiting_receiver = self.chans.get_mut(&ch).unwrap().blocked_receivers.pop_front();
        if let Some(rid) = waiting_receiver {
            self.deliver_to_receiver(rid, ch, payload);
            self.mark_ready(tid, RtPayload::Bool(true));
            return;
        }
        if self.chans[&ch].has_room() {
            self.chans.get_mut(&ch).unwrap().queue.push_back(payload);
            self.mark_ready(tid, RtPayload::Bool(true));
        } else {
            // Would block — report failure and drop the payload (never park).
            self.mark_ready(tid, RtPayload::Bool(false));
        }
    }

    fn do_try_recv(&mut self, tid: TaskId, ch: ChanId) {
        let from_queue = self.chans.get_mut(&ch).unwrap().queue.pop_front();
        if let Some(v) = from_queue {
            self.mark_ready(tid, v);
            self.refill_from_blocked_sender(ch);
            return;
        }
        let from_sender = self.chans.get_mut(&ch).unwrap().blocked_senders.pop_front();
        if let Some((sid, payload)) = from_sender {
            self.mark_ready(tid, payload);
            self.mark_ready(sid, RtPayload::Nothing);
            return;
        }
        // Empty (or closed-and-empty) — never park; resume with Nothing.
        self.mark_ready(tid, RtPayload::Nothing);
    }

    fn refill_from_blocked_sender(&mut self, ch: ChanId) {
        let take = {
            let chan = self.chans.get_mut(&ch).unwrap();
            if chan.has_room() {
                chan.blocked_senders.pop_front()
            } else {
                None
            }
        };
        if let Some((sid, payload)) = take {
            self.chans.get_mut(&ch).unwrap().queue.push_back(payload);
            self.mark_ready(sid, RtPayload::Nothing);
        }
    }

    fn deliver_to_receiver(&mut self, rid: TaskId, ch: ChanId, value: RtPayload) {
        let select_arms = self.tasks.get_mut(&rid).unwrap().select_arms.take();
        if let Some(arms) = select_arms {
            let arm_idx = arms
                .iter()
                .position(|a| matches!(a, SelectArm::Recv(c) if *c == ch))
                .expect("delivering channel must be one of the select arms");
            {
                let slot = self.tasks.get_mut(&rid).unwrap();
                slot.resume = value;
                slot.selected_arm = Some(arm_idx);
                slot.state = TaskState::Ready;
            }
            self.ready.push(rid);
            self.cancel_select_registrations(rid, Some(ch));
        } else {
            self.mark_ready(rid, value);
        }
    }

    fn cancel_select_registrations(&mut self, rid: TaskId, except: Option<ChanId>) {
        for (cid, chan) in self.chans.iter_mut() {
            if Some(*cid) == except {
                continue;
            }
            chan.blocked_receivers.retain(|t| *t != rid);
        }
        self.timers.retain(|t| t.tid != rid);
    }

    fn do_select(&mut self, tid: TaskId, arms: Vec<SelectArm>) {
        let ready_arms: Vec<usize> = arms
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                SelectArm::Recv(ch) if self.chans[ch].can_recv() => Some(i),
                _ => None,
            })
            .collect();

        if !ready_arms.is_empty() {
            let w = if ready_arms.len() == 1 {
                0
            } else {
                self.chooser.decide(ChoiceKind::SelectWinner, ready_arms.len())
            };
            let arm_idx = ready_arms[w];
            let ch = match &arms[arm_idx] {
                SelectArm::Recv(ch) => *ch,
                SelectArm::Timeout(_) => unreachable!("ready arms are recv arms"),
            };
            let v = self.take_one(ch);
            {
                let slot = self.tasks.get_mut(&tid).unwrap();
                slot.resume = v;
                slot.selected_arm = Some(arm_idx);
                slot.state = TaskState::Ready;
            }
            self.ready.push(tid);
        } else {
            for (i, a) in arms.iter().enumerate() {
                match a {
                    SelectArm::Recv(ch) => {
                        self.chans.get_mut(ch).unwrap().blocked_receivers.push_back(tid);
                    }
                    SelectArm::Timeout(d) => {
                        self.timers.push(TimerEntry { fire_at: self.clock + *d, tid, arm: Some(i) });
                    }
                }
            }
            let slot = self.tasks.get_mut(&tid).unwrap();
            slot.select_arms = Some(arms);
            slot.state = TaskState::BlockedSelect;
        }
    }

    fn take_one(&mut self, ch: ChanId) -> RtPayload {
        if let Some(v) = self.chans.get_mut(&ch).unwrap().queue.pop_front() {
            self.refill_from_blocked_sender(ch);
            return v;
        }
        if let Some((sid, payload)) = self.chans.get_mut(&ch).unwrap().blocked_senders.pop_front() {
            self.mark_ready(sid, RtPayload::Nothing);
            return payload;
        }
        RtPayload::Nothing
    }

    fn advance_timers(&mut self) {
        let earliest = match self.timers.iter().map(|t| t.fire_at).min() {
            Some(e) => e,
            None => return,
        };
        self.clock = earliest;

        let mut due = Vec::new();
        let mut rest = Vec::new();
        for t in self.timers.drain(..) {
            if t.fire_at == earliest {
                due.push(t);
            } else {
                rest.push(t);
            }
        }
        self.timers = rest;

        // Seeded tie-break order among timers firing on the same tick.
        let order: Vec<usize> = if due.len() > 1 {
            let mut pool: Vec<usize> = (0..due.len()).collect();
            let mut shuffled = Vec::with_capacity(pool.len());
            while !pool.is_empty() {
                let k = self.chooser.decide(ChoiceKind::TimerTieBreak, pool.len());
                shuffled.push(pool.remove(k));
            }
            shuffled
        } else {
            vec![0]
        };

        for idx in order {
            let tid = due[idx].tid;
            let arm = due[idx].arm;
            self.fire_timer(tid, arm);
        }
    }

    fn fire_timer(&mut self, tid: TaskId, arm: Option<usize>) {
        let still_blocked = matches!(
            self.tasks.get(&tid).map(|s| &s.state),
            Some(TaskState::BlockedTimer) | Some(TaskState::BlockedSelect)
        );
        if !still_blocked {
            return;
        }
        match arm {
            Some(arm_idx) => {
                let had = self.tasks.get_mut(&tid).unwrap().select_arms.take();
                {
                    let slot = self.tasks.get_mut(&tid).unwrap();
                    slot.selected_arm = Some(arm_idx);
                    slot.resume = RtPayload::Nothing;
                    slot.state = TaskState::Ready;
                }
                self.ready.push(tid);
                if had.is_some() {
                    self.cancel_select_registrations(tid, None);
                }
            }
            None => self.mark_ready(tid, RtPayload::Nothing),
        }
    }

    fn complete_task(&mut self, tid: TaskId, result: AwaitResult) {
        let waiters = self.awaiters.remove(&tid).unwrap_or_default();
        let payload = result.payload();
        self.results.insert(tid, result);
        for w in waiters {
            self.mark_ready(w, payload.clone());
        }
    }

    fn do_await(&mut self, tid: TaskId, target: TaskId) {
        if let Some(res) = self.results.get(&target) {
            let payload = res.payload();
            self.mark_ready(tid, payload);
        } else if !self.tasks.contains_key(&target) {
            // Unknown target — treat as already gone.
            self.mark_ready(tid, RtPayload::Nothing);
        } else {
            self.awaiters.entry(target).or_default().push(tid);
            self.tasks.get_mut(&tid).unwrap().state = TaskState::BlockedAwait;
        }
    }

    fn do_abort(&mut self, target: TaskId) {
        if self.results.contains_key(&target) {
            return; // already finished
        }
        match self.tasks.get(&target) {
            None => return,
            Some(s) if matches!(s.state, TaskState::Done) => return,
            _ => {}
        }
        self.ready.retain(|t| *t != target);
        for chan in self.chans.values_mut() {
            chan.blocked_receivers.retain(|t| *t != target);
            chan.blocked_senders.retain(|(t, _)| *t != target);
        }
        self.timers.retain(|t| t.tid != target);
        // Don't let a future completion try to re-wake an aborted task.
        for list in self.awaiters.values_mut() {
            list.retain(|t| *t != target);
        }
        self.tasks.get_mut(&target).unwrap().state = TaskState::Done;
        self.complete_task(target, AwaitResult::Aborted);
    }

    fn do_close(&mut self, ch: ChanId) {
        let waiters: Vec<TaskId> = {
            let chan = self.chans.get_mut(&ch).unwrap();
            chan.closed = true;
            chan.blocked_receivers.drain(..).collect()
        };
        for rid in waiters {
            self.wake_closed_receiver(rid, ch);
        }
    }

    fn wake_closed_receiver(&mut self, rid: TaskId, ch: ChanId) {
        let select_arms = self.tasks.get_mut(&rid).unwrap().select_arms.take();
        if let Some(arms) = select_arms {
            let arm_idx = arms
                .iter()
                .position(|a| matches!(a, SelectArm::Recv(c) if *c == ch))
                .unwrap_or(0);
            {
                let slot = self.tasks.get_mut(&rid).unwrap();
                slot.resume = RtPayload::Nothing;
                slot.selected_arm = Some(arm_idx);
                slot.state = TaskState::Ready;
            }
            self.ready.push(rid);
            self.cancel_select_registrations(rid, Some(ch));
        } else {
            self.mark_ready(rid, RtPayload::Nothing);
        }
    }
}

/// Run `setup` (which spawns tasks and creates channels) under a fresh recording
/// scheduler seeded by `seed`, returning the outcome and the scheduling trace.
pub fn run_with_seed<'t, F>(config: SchedulerConfig, seed: SchedSeed, setup: F) -> (RunOutcome, SchedTrace)
where
    F: FnOnce(&mut Scheduler<'t>),
{
    let mut sched = Scheduler::new(config, Chooser::record(seed));
    setup(&mut sched);
    let outcome = sched.run();
    (outcome, sched.into_trace())
}

/// Re-run `setup` under a replaying scheduler driven by `trace`, returning the outcome.
pub fn run_with_trace<'t, F>(config: SchedulerConfig, trace: SchedTrace, setup: F) -> RunOutcome
where
    F: FnOnce(&mut Scheduler<'t>),
{
    let mut sched = Scheduler::new(config, Chooser::replay(trace));
    setup(&mut sched);
    sched.run()
}
