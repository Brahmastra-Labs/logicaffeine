//! The async↔scheduler bridge.
//!
//! The tree-walker executes statements in an `async fn`. To run concurrent tasks
//! on the (synchronous, poll-based) [`logicaffeine_runtime`] scheduler, each
//! concurrency op writes a [`BlockingRequest`] into a per-task side-channel
//! ([`YieldState`]) and `.await`s a [`YieldFuture`], which returns `Poll::Pending`
//! once — suspending the interpreter's async stack at exactly that point. The
//! task's driver ([`super::driver::InterpreterTask`]) reads the request, maps it
//! to a scheduler `TaskStep`, and on the next poll delivers the scheduler's
//! resume value back through the same channel.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use logicaffeine_runtime::{ChanId, RtPayload, SelectArm, Task, TaskId};

/// A blocking (or non-blocking-but-scheduler-routed) request the interpreter
/// hands to the scheduler.
pub enum BlockingRequest<'a> {
    /// Create a channel of the given capacity (`None` = config default).
    NewChan(Option<usize>),
    /// Send a value into a channel (blocks if full).
    Send(ChanId, RtPayload),
    /// Non-blocking send; resume with `Payload(Bool(success))`.
    TrySend(ChanId, RtPayload),
    /// Receive a value from a channel (blocks if empty).
    Recv(ChanId),
    /// Non-blocking receive; resume with the value, or `Payload(Nothing)` if empty.
    TryRecv(ChanId),
    /// Await the first ready arm of a select.
    Select(Vec<SelectArm>),
    /// Sleep for some logical ticks.
    Sleep(u64),
    /// Spawn a child task; resume with its `TaskId`.
    Spawn(Box<dyn Task<'a> + 'a>),
    /// Await a task's completion; resume with its result payload.
    Await(TaskId),
    /// Abort a task.
    Abort(TaskId),
    /// Close a channel.
    Close(ChanId),
}

/// What the scheduler delivers back to the interpreter on resume.
pub enum ResumeValue {
    /// Nothing meaningful (e.g. after `Send`/`Sleep`/`Spawn`-fire-and-forget).
    None,
    /// A received value (after `Recv` / a winning select recv arm).
    Payload(RtPayload),
    /// A freshly-created channel id (after `NewChan`).
    Chan(ChanId),
    /// A spawned task's id (after `Spawn`).
    Task(TaskId),
    /// The winning arm of a `Select`: its index plus the received payload
    /// (`Nothing` for a timeout arm).
    Select { arm: usize, payload: RtPayload },
}

impl ResumeValue {
    /// The received payload, or `Nothing`.
    pub fn into_payload(self) -> RtPayload {
        match self {
            ResumeValue::Payload(p) => p,
            _ => RtPayload::Nothing,
        }
    }
    /// The created channel id, if this resume was from a `NewChan`.
    pub fn into_chan(self) -> Option<ChanId> {
        match self {
            ResumeValue::Chan(c) => Some(c),
            _ => None,
        }
    }
    /// The spawned task id, if this resume was from a `Spawn`.
    pub fn into_task(self) -> Option<TaskId> {
        match self {
            ResumeValue::Task(t) => Some(t),
            _ => None,
        }
    }
    /// The winning arm index + its payload, if this resume was from a `Select`.
    pub fn into_select(self) -> Option<(usize, RtPayload)> {
        match self {
            ResumeValue::Select { arm, payload } => Some((arm, payload)),
            _ => None,
        }
    }
}

/// The per-task side-channel: the interpreter writes a `request`, the driver
/// delivers a `resume`.
pub struct YieldState<'a> {
    pub request: Option<BlockingRequest<'a>>,
    pub resume: ResumeValue,
}

impl<'a> YieldState<'a> {
    pub fn new() -> Self {
        YieldState { request: None, resume: ResumeValue::None }
    }
}

impl<'a> Default for YieldState<'a> {
    fn default() -> Self {
        YieldState::new()
    }
}

/// A shared handle to a task's side-channel (cloned between the interpreter and
/// its driving `InterpreterTask`).
pub type Yield<'a> = Rc<RefCell<YieldState<'a>>>;

/// The future a concurrency op `.await`s: it returns `Poll::Pending` exactly once
/// (the request was written just before), suspending the interpreter's async
/// stack; the next poll (driven by the scheduler) resolves to the resume value.
pub struct YieldFuture<'a> {
    yielded: bool,
    ys: Yield<'a>,
}

impl<'a> YieldFuture<'a> {
    pub fn new(ys: Yield<'a>) -> Self {
        YieldFuture { yielded: false, ys }
    }
}

impl<'a> Future for YieldFuture<'a> {
    type Output = ResumeValue;
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<ResumeValue> {
        if !self.yielded {
            self.yielded = true;
            Poll::Pending
        } else {
            let mut st = self.ys.borrow_mut();
            Poll::Ready(std::mem::replace(&mut st.resume, ResumeValue::None))
        }
    }
}
