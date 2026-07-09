//! The interpreter task driver.
//!
//! [`InterpreterTask`] wraps one suspended interpreter future (an `async`
//! execution that owns its own [`crate::interpreter::Interpreter`]) as a
//! scheduler [`Task`]. On each `poll` it delivers the scheduler's resume value
//! into the side-channel, drives the future to its next suspension, and maps the
//! interpreter's [`BlockingRequest`] to a scheduler [`TaskStep`].

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use logicaffeine_runtime::{RtPayload, Task, TaskCtx, TaskStep};

use super::bridge::{BlockingRequest, ResumeValue, Yield};

/// A shared cell the first failing task writes its error into.
pub type ErrSink = Rc<RefCell<Option<String>>>;

/// Drives one interpreter task on the scheduler.
pub struct InterpreterTask<'a> {
    fut: Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>,
    ys: Yield<'a>,
    err_sink: Option<ErrSink>,
}

impl<'a> InterpreterTask<'a> {
    /// Wrap a suspended interpreter future + its side-channel. `err_sink`, when
    /// set, receives the task's error if it fails (first error wins).
    pub fn new(
        fut: Pin<Box<dyn Future<Output = Result<(), String>> + 'a>>,
        ys: Yield<'a>,
        err_sink: Option<ErrSink>,
    ) -> Self {
        InterpreterTask { fut, ys, err_sink }
    }
}

impl<'a> Task<'a> for InterpreterTask<'a> {
    fn poll(&mut self, ctx: &mut TaskCtx) -> TaskStep<'a> {
        // Deliver whatever the scheduler resumed us with into the side-channel,
        // so the `.await`ing concurrency op resolves to it.
        {
            let mut st = self.ys.borrow_mut();
            st.resume = ctx_to_resume(ctx);
        }
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        match self.fut.as_mut().poll(&mut cx) {
            Poll::Ready(result) => {
                if let Err(e) = result {
                    if let Some(sink) = &self.err_sink {
                        let mut slot = sink.borrow_mut();
                        if slot.is_none() {
                            *slot = Some(e);
                        }
                    }
                }
                TaskStep::Exit(RtPayload::Nothing)
            }
            Poll::Pending => {
                // A `Some` request is a deliberate concurrency yield (channel/timer/select).
                // A `None` means the interpreter is awaiting real external I/O — a network
                // op (`Connect`/`Listen`/peer messaging) whose future is pending on the host
                // reactor. The scheduler can't service that itself, so park the task
                // `IoPending`; the async drive loop yields to the reactor and re-polls it.
                match self.ys.borrow_mut().request.take() {
                    Some(req) => request_to_taskstep(req),
                    None => TaskStep::IoPending,
                }
            }
        }
    }
}

/// Translate the scheduler's resume context into the interpreter's resume value.
fn ctx_to_resume(ctx: &mut TaskCtx) -> ResumeValue {
    if let Some(arm) = ctx.selected_arm.take() {
        ResumeValue::Select {
            arm,
            payload: std::mem::replace(&mut ctx.resumed_with, RtPayload::Nothing),
        }
    } else if let Some(id) = ctx.new_chan {
        ResumeValue::Chan(id)
    } else if let Some(id) = ctx.spawned {
        ResumeValue::Task(id)
    } else {
        ResumeValue::Payload(std::mem::replace(&mut ctx.resumed_with, RtPayload::Nothing))
    }
}

/// Map an interpreter blocking request to the scheduler step that services it.
fn request_to_taskstep<'a>(req: BlockingRequest<'a>) -> TaskStep<'a> {
    match req {
        BlockingRequest::NewChan(c) => TaskStep::NewChan(c),
        BlockingRequest::Send(ch, p) => TaskStep::Send(ch, p),
        BlockingRequest::TrySend(ch, p) => TaskStep::TrySend(ch, p),
        BlockingRequest::Recv(ch) => TaskStep::Recv(ch),
        BlockingRequest::TryRecv(ch) => TaskStep::TryRecv(ch),
        BlockingRequest::Select(arms) => TaskStep::Select(arms),
        BlockingRequest::Sleep(d) => TaskStep::Sleep(d),
        BlockingRequest::Spawn(t) => TaskStep::Spawn(t),
        BlockingRequest::Await(t) => TaskStep::Await(t),
        BlockingRequest::Abort(t) => TaskStep::Abort(t),
        BlockingRequest::Close(ch) => TaskStep::Close(ch),
    }
}
