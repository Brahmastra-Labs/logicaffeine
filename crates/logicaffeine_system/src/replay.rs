//! Deterministic replay: the executor that programs compiled by
//! `logicaffeine_compile::compile_to_rust_deterministic` run their tasks on.
//!
//! The interpreter and the VM run every task, pipe, timer and select on `logicaffeine_runtime`'s
//! scheduler — one FIFO ready queue on a logical clock, where a launched task runs before its parent
//! resumes and every concurrency operation is a scheduling point. Compiled binaries do not link that
//! crate, so this module is its port: the same state machine, driving Rust futures
//! instead of interpreter continuations, drawing every choice from the same seeded chooser
//! ([`crate::concurrency::seeded_pick`]) in the same order. A program compiled this way and run at
//! `LOGOS_SEED=s` prints what the interpreter prints at seed `s`.
//!
//! A task is a future. Each operation is an `async fn` that hands the executor one request and
//! suspends exactly once; the executor services it by the scheduler's rules and resumes the task with
//! the answer. A task that suspends without a request is waiting on host I/O (a socket): it is parked
//! until the host reactor wakes the run, then re-polled, as the interpreter's async driver does.
//!
//! Values cross a pipe by move. The interpreter copies a value into a channel and rebuilds it on the
//! far side; a compiled value is already an owned copy (the lowering clones a named value it sends),
//! so moving it is the same observation. A task's result is not propagated: awaiting a handle yields
//! nothing, on every tier.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::marker::PhantomData;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// The default capacity of a pipe created without one — the scheduler's
/// `SchedulerConfig::default().default_channel_capacity`.
pub const DEFAULT_PIPE_CAPACITY: usize = 32;

type Body = Pin<Box<dyn Future<Output = ()>>>;
type Payload = Option<Box<dyn Any>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TaskId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChanId(u64);

/// One arm of a select: a receive from a pipe, or a timeout after some logical ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arm(ArmKind);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArmKind {
    Recv(ChanId),
    After(u64),
}

impl Arm {
    /// `After d:` — fires after `d` logical ticks unless a receive arm wins first.
    pub fn after(ticks: impl TimeoutTicks) -> Arm {
        Arm(ArmKind::After(ticks.timeout_ticks()))
    }
}

/// A logical duration for `Sleep`: an `Int` is that many ticks, a `Duration` its nanoseconds — the
/// interpreter's reading of the same values.
pub trait Ticks {
    fn ticks(self) -> u64;
}

impl Ticks for i64 {
    fn ticks(self) -> u64 {
        self.max(0) as u64
    }
}

impl Ticks for std::time::Duration {
    fn ticks(self) -> u64 {
        u64::try_from(self.as_nanos()).unwrap_or(u64::MAX)
    }
}

/// A logical duration for a select's `After` arm: an `Int` or a `Duration` as for [`Ticks`], and a
/// calendar span as whole seconds (a month is thirty days).
pub trait TimeoutTicks {
    fn timeout_ticks(self) -> u64;
}

impl TimeoutTicks for i64 {
    fn timeout_ticks(self) -> u64 {
        self.ticks()
    }
}

impl TimeoutTicks for std::time::Duration {
    fn timeout_ticks(self) -> u64 {
        self.ticks()
    }
}

impl TimeoutTicks for crate::temporal::LogosSpan {
    fn timeout_ticks(self) -> u64 {
        ((self.months as i64 * 30 + self.days as i64) * 86_400).max(0) as u64
    }
}

/// A pipe of `T` values — a handle to one scheduler channel. Copying a handle never copies the pipe.
pub struct Pipe<T> {
    id: ChanId,
    elem: PhantomData<fn(T) -> T>,
}

impl<T> Clone for Pipe<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Pipe<T> {}

impl<T> std::fmt::Debug for Pipe<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pipe({})", self.id.0)
    }
}

impl<T: 'static> Pipe<T> {
    /// `Let ch be a Pipe of T` — a scheduling point, like every operation on a pipe.
    pub async fn open(capacity: Option<usize>) -> Pipe<T> {
        let resume = Op::new(Request::NewChan(capacity)).await;
        Pipe {
            id: resume.chan.expect("the executor creates the pipe"),
            elem: PhantomData,
        }
    }

    /// `Send v into ch` — blocks while the pipe is full.
    pub async fn send(self, value: T) {
        Op::new(Request::Send(self.id, Box::new(value))).await;
    }

    /// `Try to send v into ch` — whether the pipe took the value; a full pipe drops it.
    pub async fn try_send(self, value: T) -> bool {
        let resume = Op::new(Request::TrySend(self.id, Box::new(value))).await;
        resume
            .payload
            .and_then(|p| p.downcast::<bool>().ok())
            .map_or(false, |ok| *ok)
    }

    /// `Receive x from ch` — blocks while the pipe is empty and open.
    pub async fn recv(self) -> T {
        closed_and_drained(self.recv_or_closed().await)
    }

    /// [`Pipe::recv`], with `None` for a pipe that is closed and has no value left — where the
    /// scheduler resumes the receiver with nothing.
    pub async fn recv_or_closed(self) -> Option<T> {
        let resume = Op::new(Request::Recv(self.id)).await;
        downcast(resume.payload)
    }

    /// `Try to receive x from ch` — a value if one is ready, nothing otherwise.
    pub async fn try_recv(self) -> Option<T> {
        let resume = Op::new(Request::TryRecv(self.id)).await;
        downcast(resume.payload)
    }

    /// `Close ch` — a receive on the drained pipe no longer blocks.
    pub async fn close(self) {
        Op::new(Request::Close(self.id)).await;
    }

    /// This pipe as a select's receive arm.
    pub fn arm(self) -> Arm {
        Arm(ArmKind::Recv(self.id))
    }

    /// The value a select's winning receive arm on this pipe delivered.
    pub fn received(self, got: Received) -> T {
        closed_and_drained(self.received_or_closed(got))
    }

    /// [`Pipe::received`], with `None` when the arm won because the pipe closed with no value left.
    pub fn received_or_closed(self, got: Received) -> Option<T> {
        downcast(got.0)
    }
}

fn downcast<T: 'static>(payload: Payload) -> Option<T> {
    payload.map(|p| {
        *p.downcast::<T>()
            .expect("a pipe carries its own element type")
    })
}

/// A compiled receive binds a value of the pipe's element type, which has no `nothing` to stand for a
/// closed, drained pipe — the one delivery the interpreter binds as `nothing`. It stops loudly.
fn closed_and_drained<T>(got: Option<T>) -> T {
    got.unwrap_or_else(|| panic!("Receive from a closed pipe that has no value left"))
}

/// What a select's winning arm delivered — a value for a receive arm, nothing for a timeout.
pub struct Received(Payload);

/// A launched task's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Task(TaskId);

impl Task {
    /// `Stop handle` — the task never runs again; its awaiters wake.
    pub async fn stop(self) {
        Op::new(Request::Abort(self.0)).await;
    }

    /// `Await handle` — blocks until the task finishes or is stopped. A task's result is not
    /// propagated (the interpreter and the VM both deliver nothing), so the value is `nothing`.
    pub async fn join(self) -> logicaffeine_data::Value {
        Op::new(Request::Await(self.0)).await;
        logicaffeine_data::Value::Nothing
    }
}

/// `Launch a task to …` — the child runs before its parent resumes.
pub async fn spawn<F: Future<Output = ()> + 'static>(task: F) -> Task {
    let resume = Op::new(Request::Spawn(Box::pin(task))).await;
    Task(resume.task.expect("the executor spawns the task"))
}

/// `Sleep d` on the logical clock. A zero sleep is no scheduling point at all.
pub async fn sleep(duration: impl Ticks) {
    let ticks = duration.ticks();
    if ticks > 0 {
        Op::new(Request::Sleep(ticks)).await;
    }
}

/// The arms of `arms` that could be taken right now, in declaration order: a receive arm whose pipe
/// has a value, a blocked sender, or is closed. A timeout arm is never ready. The lowering draws the
/// winner among these with [`crate::concurrency::seeded_pick`] and hands it to [`select`] in the
/// same step, exactly where the scheduler draws it.
pub fn ready_arms(arms: &[Arm]) -> Vec<usize> {
    with_executor(|e| {
        arms.iter()
            .enumerate()
            .filter_map(|(i, arm)| match arm.0 {
                ArmKind::Recv(ch) if e.chans[&ch].can_recv() => Some(i),
                _ => None,
            })
            .collect()
    })
}

/// `Await the first of:` — resumes with the winning arm's index and what it delivered. `pick` is the
/// winner already drawn among [`ready_arms`]; with none ready the task blocks on every arm at once.
pub async fn select(arms: &[Arm], pick: Option<usize>) -> (usize, Received) {
    let resume = Op::new(Request::Select(arms.to_vec(), pick)).await;
    (
        resume.selected.expect("the executor resolves the select"),
        Received(resume.payload),
    )
}

/// Why a run failed. A task's failure has already reported itself through the panic hook; a
/// deadlock is reported by its [`std::fmt::Display`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// A task failed — the main task or one it launched. The others still ran to quiescence, as
    /// the VM runs them, and the run reports the failure.
    Task,
    /// Every task is blocked and nothing can wake one.
    Deadlock,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Task => write!(f, "a task failed"),
            Failure::Deadlock => write!(f, "deadlock: every task is blocked waiting"),
        }
    }
}

/// Run `main` as the main task, and every task it launches, to quiescence, under the seed
/// `LOGOS_SEED` names (default 0). Drive it with any executor: it returns `Pending` only while a task
/// waits on host I/O, having registered the host's waker with that I/O.
pub fn run<F: Future<Output = ()> + 'static>(main: F) -> Run {
    run_seeded(crate::concurrency::logos_seed(), main)
}

/// [`run`] under an explicit scheduling seed.
pub fn run_seeded<F: Future<Output = ()> + 'static>(seed: u64, main: F) -> Run {
    Run {
        main: Some(Box::pin(main)),
        seed,
        installed: false,
    }
}

/// The future [`run`] returns.
pub struct Run {
    main: Option<Body>,
    seed: u64,
    installed: bool,
}

impl Future for Run {
    type Output = Result<(), Failure>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(main) = self.main.take() {
            EXECUTOR.with(|slot| {
                let mut slot = slot.borrow_mut();
                assert!(
                    slot.is_none(),
                    "a deterministic run is already in progress on this thread"
                );
                let mut executor = Executor::new(self.seed);
                executor.spawn_task(main);
                *slot = Some(executor);
            });
            self.installed = true;
        } else {
            with_executor(Executor::wake_io);
        }
        loop {
            match poll_once(cx.waker()) {
                Step::Progress => {}
                Step::WaitingForIo => return Poll::Pending,
                Step::Finished(outcome) => {
                    EXECUTOR.with(|slot| slot.borrow_mut().take());
                    self.installed = false;
                    return Poll::Ready(outcome);
                }
            }
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if self.installed {
            let executor = EXECUTOR.with(|slot| slot.borrow_mut().take());
            drop(executor);
        }
    }
}

thread_local! {
    static EXECUTOR: RefCell<Option<Executor>> = const { RefCell::new(None) };
}

/// A draw from the running replay's chooser — `None` outside a run. [`crate::concurrency::seeded_pick`]
/// routes through it, so a compiled select and the timer tie-breaks share one decision stream.
pub(crate) fn run_pick(n: usize) -> Option<usize> {
    EXECUTOR.with(|slot| slot.borrow_mut().as_mut().map(|e| e.chooser.below(n)))
}

fn with_executor<R>(f: impl FnOnce(&mut Executor) -> R) -> R {
    EXECUTOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        let executor = slot
            .as_mut()
            .expect("a concurrency operation ran outside a deterministic run");
        f(executor)
    })
}

enum Step {
    Progress,
    WaitingForIo,
    Finished(Result<(), Failure>),
}

/// One scheduler step: poll one ready task, or advance the timers, or report quiescence. The task's
/// body is taken out of the executor while it runs, since its operations reach the executor too.
fn poll_once(waker: &Waker) -> Step {
    let picked = with_executor(|e| {
        let tid = e.ready.pop_front()?;
        let body = e.bodies.remove(&tid).expect("a ready task has a body");
        e.staged = std::mem::take(
            &mut e
                .slots
                .get_mut(&tid)
                .expect("a ready task has a slot")
                .resume,
        );
        e.request = None;
        Some((tid, body))
    });
    let Some((tid, mut body)) = picked else {
        return with_executor(|e| {
            if !e.timers.is_empty() {
                e.advance_timers();
                Step::Progress
            } else if e.slots.values().all(|s| s.state == State::Done) {
                Step::Finished(if e.failed { Err(Failure::Task) } else { Ok(()) })
            } else if e.slots.values().any(|s| s.state == State::BlockedIo) {
                Step::WaitingForIo
            } else {
                Step::Finished(Err(if e.failed {
                    Failure::Task
                } else {
                    Failure::Deadlock
                }))
            }
        });
    };
    let mut cx = Context::from_waker(waker);
    let polled = crate::failure::caught(AssertUnwindSafe(|| body.as_mut().poll(&mut cx)));
    with_executor(|e| {
        let request = e.request.take();
        match polled {
            Ok(Poll::Pending) => match request {
                Some(request) => {
                    e.process(tid, request);
                    if e.slots[&tid].state != State::Done {
                        e.bodies.insert(tid, body);
                    }
                }
                None => {
                    e.bodies.insert(tid, body);
                    e.slots
                        .get_mut(&tid)
                        .expect("the polled task has a slot")
                        .state = State::BlockedIo;
                }
            },
            Ok(Poll::Ready(())) => e.exit(tid),
            Err(_) => {
                e.failed = true;
                e.exit(tid);
            }
        }
    });
    Step::Progress
}

/// A request a task hands the executor before it suspends.
enum Request {
    NewChan(Option<usize>),
    Send(ChanId, Box<dyn Any>),
    TrySend(ChanId, Box<dyn Any>),
    Recv(ChanId),
    TryRecv(ChanId),
    Select(Vec<Arm>, Option<usize>),
    Sleep(u64),
    Spawn(Body),
    Await(TaskId),
    Abort(TaskId),
    Close(ChanId),
}

/// What the executor resumes a task with.
#[derive(Default)]
struct Resume {
    payload: Payload,
    selected: Option<usize>,
    chan: Option<ChanId>,
    task: Option<TaskId>,
}

/// The one suspension every operation makes: hand over the request, return `Pending` once, and
/// resolve to what the executor staged for this task's next poll.
struct Op {
    request: Option<Request>,
}

impl Op {
    fn new(request: Request) -> Op {
        Op {
            request: Some(request),
        }
    }
}

impl Future for Op {
    type Output = Resume;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Resume> {
        match self.request.take() {
            Some(request) => {
                with_executor(|e| {
                    assert!(
                        e.request.is_none(),
                        "a task made two scheduling requests in one step"
                    );
                    e.request = Some(request);
                });
                Poll::Pending
            }
            None => Poll::Ready(with_executor(|e| std::mem::take(&mut e.staged))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    BlockedRecv,
    BlockedSend,
    BlockedSelect,
    BlockedTimer,
    BlockedAwait,
    BlockedIo,
    Done,
}

struct Slot {
    state: State,
    resume: Resume,
    select_arms: Option<Vec<Arm>>,
}

struct Chan {
    capacity: usize,
    queue: VecDeque<Box<dyn Any>>,
    blocked_senders: VecDeque<(TaskId, Box<dyn Any>)>,
    blocked_receivers: VecDeque<TaskId>,
    closed: bool,
}

impl Chan {
    fn has_room(&self) -> bool {
        self.queue.len() < self.capacity
    }

    fn can_recv(&self) -> bool {
        !self.queue.is_empty() || !self.blocked_senders.is_empty() || self.closed
    }
}

struct Timer {
    fire_at: u64,
    tid: TaskId,
    arm: Option<usize>,
}

struct Executor {
    chooser: crate::concurrency::SplitMix64,
    slots: HashMap<TaskId, Slot>,
    bodies: HashMap<TaskId, Body>,
    ready: VecDeque<TaskId>,
    chans: HashMap<ChanId, Chan>,
    timers: Vec<Timer>,
    clock: u64,
    next_id: u64,
    finished: HashSet<TaskId>,
    awaiters: HashMap<TaskId, Vec<TaskId>>,
    failed: bool,
    /// The request the running task made in this step.
    request: Option<Request>,
    /// The resume value of the task being polled, taken by its pending operation.
    staged: Resume,
}

impl Executor {
    fn new(seed: u64) -> Executor {
        Executor {
            chooser: crate::concurrency::SplitMix64::new(seed),
            slots: HashMap::new(),
            bodies: HashMap::new(),
            ready: VecDeque::new(),
            chans: HashMap::new(),
            timers: Vec::new(),
            clock: 0,
            next_id: 0,
            finished: HashSet::new(),
            awaiters: HashMap::new(),
            failed: false,
            request: None,
            staged: Resume::default(),
        }
    }

    fn bump(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn spawn_task(&mut self, body: Body) -> TaskId {
        let id = TaskId(self.bump());
        self.slots.insert(
            id,
            Slot {
                state: State::Ready,
                resume: Resume::default(),
                select_arms: None,
            },
        );
        self.bodies.insert(id, body);
        self.ready.push_back(id);
        id
    }

    fn slot(&mut self, tid: TaskId) -> &mut Slot {
        self.slots
            .get_mut(&tid)
            .expect("a scheduled task has a slot")
    }

    fn chan(&mut self, ch: ChanId) -> &mut Chan {
        self.chans
            .get_mut(&ch)
            .expect("a pipe handle names a live pipe")
    }

    fn mark_ready(&mut self, tid: TaskId, payload: Payload) {
        let slot = self.slot(tid);
        slot.resume.payload = payload;
        slot.state = State::Ready;
        self.ready.push_back(tid);
    }

    fn wake_io(&mut self) {
        let mut parked: Vec<TaskId> = self
            .slots
            .iter()
            .filter(|(_, s)| s.state == State::BlockedIo)
            .map(|(id, _)| *id)
            .collect();
        parked.sort();
        for tid in parked {
            self.slot(tid).state = State::Ready;
            self.ready.push_back(tid);
        }
    }

    fn exit(&mut self, tid: TaskId) {
        self.bodies.remove(&tid);
        self.slot(tid).state = State::Done;
        self.complete(tid);
    }

    fn complete(&mut self, tid: TaskId) {
        let waiters = self.awaiters.remove(&tid).unwrap_or_default();
        self.finished.insert(tid);
        for waiter in waiters {
            self.mark_ready(waiter, None);
        }
    }

    fn process(&mut self, tid: TaskId, request: Request) {
        match request {
            Request::Await(target) => self.await_task(tid, target),
            Request::Abort(target) => {
                self.abort(target);
                self.mark_ready(tid, None);
            }
            Request::Close(ch) => {
                self.close(ch);
                self.mark_ready(tid, None);
            }
            Request::Spawn(child) => {
                let child = self.spawn_task(child);
                self.slot(tid).resume.task = Some(child);
                self.mark_ready(tid, None);
            }
            Request::NewChan(capacity) => {
                let id = ChanId(self.bump());
                self.chans.insert(
                    id,
                    Chan {
                        capacity: capacity.unwrap_or(DEFAULT_PIPE_CAPACITY),
                        queue: VecDeque::new(),
                        blocked_senders: VecDeque::new(),
                        blocked_receivers: VecDeque::new(),
                        closed: false,
                    },
                );
                self.slot(tid).resume.chan = Some(id);
                self.mark_ready(tid, None);
            }
            Request::Send(ch, value) => self.send(tid, ch, value),
            Request::TrySend(ch, value) => self.try_send(tid, ch, value),
            Request::Recv(ch) => self.recv(tid, ch),
            Request::TryRecv(ch) => self.try_recv(tid, ch),
            Request::Sleep(ticks) => {
                self.timers.push(Timer {
                    fire_at: self.clock + ticks,
                    tid,
                    arm: None,
                });
                self.slot(tid).state = State::BlockedTimer;
            }
            Request::Select(arms, pick) => self.select(tid, arms, pick),
        }
    }

    fn send(&mut self, tid: TaskId, ch: ChanId, value: Box<dyn Any>) {
        if let Some(receiver) = self.chan(ch).blocked_receivers.pop_front() {
            self.deliver(receiver, ch, Some(value));
            self.mark_ready(tid, None);
        } else if self.chan(ch).has_room() {
            self.chan(ch).queue.push_back(value);
            self.mark_ready(tid, None);
        } else {
            self.chan(ch).blocked_senders.push_back((tid, value));
            self.slot(tid).state = State::BlockedSend;
        }
    }

    fn try_send(&mut self, tid: TaskId, ch: ChanId, value: Box<dyn Any>) {
        if let Some(receiver) = self.chan(ch).blocked_receivers.pop_front() {
            self.deliver(receiver, ch, Some(value));
            self.mark_ready(tid, Some(Box::new(true)));
        } else if self.chan(ch).has_room() {
            self.chan(ch).queue.push_back(value);
            self.mark_ready(tid, Some(Box::new(true)));
        } else {
            self.mark_ready(tid, Some(Box::new(false)));
        }
    }

    fn recv(&mut self, tid: TaskId, ch: ChanId) {
        if let Some(value) = self.chan(ch).queue.pop_front() {
            self.mark_ready(tid, Some(value));
            self.refill_from_blocked_sender(ch);
        } else if let Some((sender, value)) = self.chan(ch).blocked_senders.pop_front() {
            self.mark_ready(tid, Some(value));
            self.mark_ready(sender, None);
        } else if self.chan(ch).closed {
            self.mark_ready(tid, None);
        } else {
            self.chan(ch).blocked_receivers.push_back(tid);
            self.slot(tid).state = State::BlockedRecv;
        }
    }

    fn try_recv(&mut self, tid: TaskId, ch: ChanId) {
        if let Some(value) = self.chan(ch).queue.pop_front() {
            self.mark_ready(tid, Some(value));
            self.refill_from_blocked_sender(ch);
        } else if let Some((sender, value)) = self.chan(ch).blocked_senders.pop_front() {
            self.mark_ready(tid, Some(value));
            self.mark_ready(sender, None);
        } else {
            self.mark_ready(tid, None);
        }
    }

    fn refill_from_blocked_sender(&mut self, ch: ChanId) {
        let chan = self.chan(ch);
        let taken = if chan.has_room() {
            chan.blocked_senders.pop_front()
        } else {
            None
        };
        if let Some((sender, value)) = taken {
            self.chan(ch).queue.push_back(value);
            self.mark_ready(sender, None);
        }
    }

    /// Hand `value` to a task blocked receiving on `ch` — plainly, or as the winning arm of its select.
    fn deliver(&mut self, receiver: TaskId, ch: ChanId, value: Payload) {
        match self.slot(receiver).select_arms.take() {
            Some(arms) => {
                let arm = arms
                    .iter()
                    .position(|a| a.0 == ArmKind::Recv(ch))
                    .expect("the delivering pipe is one of the select's arms");
                let slot = self.slot(receiver);
                slot.resume.payload = value;
                slot.resume.selected = Some(arm);
                slot.state = State::Ready;
                self.ready.push_back(receiver);
                self.cancel_select(receiver);
            }
            None => self.mark_ready(receiver, value),
        }
    }

    /// Withdraw every registration a woken select made — on every pipe, the one that woke it too,
    /// since a select naming one pipe in two arms waits on it twice — and its timeouts.
    fn cancel_select(&mut self, tid: TaskId) {
        for chan in self.chans.values_mut() {
            chan.blocked_receivers.retain(|t| *t != tid);
        }
        self.timers.retain(|t| t.tid != tid);
    }

    fn select(&mut self, tid: TaskId, arms: Vec<Arm>, pick: Option<usize>) {
        debug_assert_eq!(
            pick.is_some(),
            arms.iter()
                .any(|a| matches!(a.0, ArmKind::Recv(ch) if self.chans[&ch].can_recv())),
            "a select's pick is drawn exactly when an arm is ready"
        );
        match pick {
            Some(arm) => {
                let ArmKind::Recv(ch) = arms[arm].0 else {
                    unreachable!("a ready arm is a receive arm")
                };
                let value = self.take_one(ch);
                let slot = self.slot(tid);
                slot.resume.payload = value;
                slot.resume.selected = Some(arm);
                slot.state = State::Ready;
                self.ready.push_back(tid);
            }
            None => {
                for (i, arm) in arms.iter().enumerate() {
                    match arm.0 {
                        ArmKind::Recv(ch) => self.chan(ch).blocked_receivers.push_back(tid),
                        ArmKind::After(ticks) => self.timers.push(Timer {
                            fire_at: self.clock + ticks,
                            tid,
                            arm: Some(i),
                        }),
                    }
                }
                let slot = self.slot(tid);
                slot.select_arms = Some(arms);
                slot.state = State::BlockedSelect;
            }
        }
    }

    fn take_one(&mut self, ch: ChanId) -> Payload {
        if let Some(value) = self.chan(ch).queue.pop_front() {
            self.refill_from_blocked_sender(ch);
            return Some(value);
        }
        if let Some((sender, value)) = self.chan(ch).blocked_senders.pop_front() {
            self.mark_ready(sender, None);
            return Some(value);
        }
        None
    }

    fn advance_timers(&mut self) {
        let Some(earliest) = self.timers.iter().map(|t| t.fire_at).min() else {
            return;
        };
        self.clock = earliest;
        let (due, rest): (Vec<Timer>, Vec<Timer>) =
            self.timers.drain(..).partition(|t| t.fire_at == earliest);
        self.timers = rest;
        let order: Vec<usize> = if due.len() > 1 {
            let mut pool: Vec<usize> = (0..due.len()).collect();
            let mut shuffled = Vec::with_capacity(pool.len());
            while !pool.is_empty() {
                let k = self.chooser.below(pool.len());
                shuffled.push(pool.remove(k));
            }
            shuffled
        } else {
            vec![0]
        };
        for i in order {
            self.fire_timer(due[i].tid, due[i].arm);
        }
    }

    fn fire_timer(&mut self, tid: TaskId, arm: Option<usize>) {
        let blocked = matches!(
            self.slots.get(&tid).map(|s| s.state),
            Some(State::BlockedTimer) | Some(State::BlockedSelect)
        );
        if !blocked {
            return;
        }
        match arm {
            Some(arm) => {
                let had = self.slot(tid).select_arms.take();
                let slot = self.slot(tid);
                slot.resume.selected = Some(arm);
                slot.resume.payload = None;
                slot.state = State::Ready;
                self.ready.push_back(tid);
                if had.is_some() {
                    self.cancel_select(tid);
                }
            }
            None => self.mark_ready(tid, None),
        }
    }

    fn await_task(&mut self, tid: TaskId, target: TaskId) {
        if self.finished.contains(&target) || !self.slots.contains_key(&target) {
            self.mark_ready(tid, None);
        } else {
            self.awaiters.entry(target).or_default().push(tid);
            self.slot(tid).state = State::BlockedAwait;
        }
    }

    fn abort(&mut self, target: TaskId) {
        if self.finished.contains(&target) {
            return;
        }
        match self.slots.get(&target) {
            None => return,
            Some(s) if s.state == State::Done => return,
            _ => {}
        }
        self.ready.retain(|t| *t != target);
        for chan in self.chans.values_mut() {
            chan.blocked_receivers.retain(|t| *t != target);
            chan.blocked_senders.retain(|(t, _)| *t != target);
        }
        self.timers.retain(|t| t.tid != target);
        for waiting in self.awaiters.values_mut() {
            waiting.retain(|t| *t != target);
        }
        self.bodies.remove(&target);
        self.slot(target).state = State::Done;
        self.complete(target);
    }

    fn close(&mut self, ch: ChanId) {
        let chan = self.chan(ch);
        chan.closed = true;
        let waiting: Vec<TaskId> = chan.blocked_receivers.drain(..).collect();
        // A select naming this pipe in two arms is registered twice, and wakes once.
        let mut woken = HashSet::new();
        for receiver in waiting {
            if !woken.insert(receiver) {
                continue;
            }
            match self.slot(receiver).select_arms.take() {
                Some(arms) => {
                    let arm = arms
                        .iter()
                        .position(|a| a.0 == ArmKind::Recv(ch))
                        .unwrap_or(0);
                    let slot = self.slot(receiver);
                    slot.resume.payload = None;
                    slot.resume.selected = Some(arm);
                    slot.state = State::Ready;
                    self.ready.push_back(receiver);
                    self.cancel_select(receiver);
                }
                None => self.mark_ready(receiver, None),
            }
        }
    }
}
